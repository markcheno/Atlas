//! LanceDB (Lance columnar files, Arrow, DataFusion). Variants:
//!
//! - `ivfpq`: IVF-PQ index; the knob is a (`nprobes`, `refine_factor`) pair:
//!   partitions searched, then refine×k candidates re-ranked on the full
//!   vectors.
//! - `hnswsq`: IVF partitions each holding an HNSW graph over scalar-
//!   quantized vectors; the knob is HNSW `ef` (partitions: Lance's default).
//!
//! Lance's default IVF partitions hold about 8k rows each, so `nprobes`
//! only matters for large tables.
//!
//! A bitmap index on `root` serves the filter (`only_if`, prefiltered).
//! Every write is a new table version; deletes are tombstones until
//! `optimize` compacts, and old versions stay on disk until pruned.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::TryStreamExt;
use lancedb::arrow::arrow_array::types::Float32Type;
use lancedb::arrow::arrow_array::{Array, FixedSizeListArray, Int32Array, RecordBatch, UInt64Array};
use lancedb::arrow::arrow_schema::{DataType, Field, Schema};
use lancedb::index::Index;
use lancedb::index::scalar::BitmapIndexBuilder;
use lancedb::index::vector::{IvfHnswSqIndexBuilder, IvfPqIndexBuilder};
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use lancedb::table::OptimizeAction;
use lancedb::{DistanceType, Table, connect};
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use vbench::{Data, Engine, K, Knob, Result, knob};

/// (nprobes, refine_factor) settings for IVF-PQ, cheapest first.
const PQ_SETTINGS: [(usize, u32); 7] = [(5, 2), (10, 5), (20, 10), (20, 20), (50, 20), (50, 50), (100, 100)];
const BATCH: usize = 10_000;

#[derive(Clone, Copy, PartialEq)]
enum Variant {
    IvfPq,
    HnswSq,
}

struct Lance {
    dir: PathBuf,
    variant: Variant,
    label: String,
    rt: Runtime,
    table: Option<Table>,
    /// IVF-PQ: index into PQ_SETTINGS; HNSW-SQ: ef.
    setting: usize,
    dim: usize,
}

fn schema(dim: usize) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::UInt64, false),
        Field::new("root", DataType::Int32, false),
        Field::new("vector", DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim as i32), true),
    ]))
}

fn batch(data: &Data, range: std::ops::Range<usize>) -> Result<RecordBatch> {
    let dim = data.dim;
    let ids = UInt64Array::from_iter_values(range.clone().map(|i| i as u64));
    let roots = Int32Array::from_iter_values(range.clone().map(|i| data.roots[i] as i32));
    let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        range.map(|i| Some(data.vector(i).iter().map(|x| Some(*x)).collect::<Vec<_>>())),
        dim as i32,
    );
    Ok(RecordBatch::try_new(schema(dim), vec![Arc::new(ids), Arc::new(roots), Arc::new(vectors)])?)
}

impl Lance {
    fn uri(&self) -> String {
        self.dir.join("db").to_string_lossy().into_owned()
    }

    fn table(&self) -> &Table {
        self.table.as_ref().expect("table open")
    }

    fn open(&mut self) -> Result<()> {
        let uri = self.uri();
        let table = self.rt.block_on(async move {
            let db = connect(&uri).execute().await?;
            db.open_table("v").execute().await
        })?;
        self.table = Some(table);
        Ok(())
    }

    fn create(&mut self, first: RecordBatch) -> Result<()> {
        let uri = self.uri();
        let table = self.rt.block_on(async move {
            let db = connect(&uri).execute().await?;
            db.create_table("v", first).execute().await
        })?;
        self.table = Some(table);
        Ok(())
    }
}

impl Engine for Lance {
    fn build(&mut self, data: &Data) -> Result<()> {
        self.dim = data.dim;
        let n = data.n();
        let batches: Vec<RecordBatch> =
            (0..n).step_by(BATCH).map(|s| batch(data, s..(s + BATCH).min(n))).collect::<Result<_>>()?;
        let uri = self.uri();
        let variant = self.variant;
        let table = self.rt.block_on(async move {
            let db = connect(&uri).execute().await?;
            let table = db.create_table("v", batches).execute().await?;
            let index = match variant {
                Variant::IvfPq => Index::IvfPq(IvfPqIndexBuilder::default().distance_type(DistanceType::Cosine)),
                Variant::HnswSq => Index::IvfHnswSq(IvfHnswSqIndexBuilder::default().distance_type(DistanceType::Cosine)),
            };
            table.create_index(&["vector"], index).execute().await?;
            table.create_index(&["root"], Index::Bitmap(BitmapIndexBuilder::default())).execute().await?;
            lancedb::Result::Ok(table)
        })?;
        self.table = Some(table);
        Ok(())
    }

    fn reopen(&mut self) -> Result<()> {
        self.table = None;
        self.open()
    }

    fn knobs(&self) -> Vec<Knob> {
        match self.variant {
            Variant::IvfPq => PQ_SETTINGS
                .iter()
                .enumerate()
                .map(|(i, (p, r))| knob(&format!("nprobes={p} refine={r}"), i as f64))
                .collect(),
            Variant::HnswSq => [10.0, 20.0, 40.0, 80.0, 160.0, 320.0].iter().map(|v| knob("ef", *v)).collect(),
        }
    }

    fn set_knob(&mut self, k: &Knob) -> Result<()> {
        self.setting = k.value as usize;
        Ok(())
    }

    fn search(&mut self, q: &[f32], k: usize, root: Option<u16>) -> Result<Vec<u64>> {
        let table = self.table().clone();
        let q = q.to_vec();
        let setting = self.setting;
        let variant = self.variant;
        let ids = self.rt.block_on(async move {
            let mut query = table
                .vector_search(q.as_slice())?
                .distance_type(DistanceType::Cosine)
                .limit(k)
                .select(Select::Columns(vec!["id".into()]));
            match variant {
                Variant::IvfPq => {
                    let (nprobes, refine) = PQ_SETTINGS[setting.min(PQ_SETTINGS.len() - 1)];
                    query = query.minimum_nprobes(nprobes)?.maximum_nprobes(Some(nprobes))?.refine_factor(refine);
                }
                Variant::HnswSq => query = query.ef(setting),
            }
            if let Some(r) = root {
                query = query.only_if(format!("root = {r}"));
            }
            let batches: Vec<RecordBatch> = query.execute().await?.try_collect().await?;
            let mut ids = Vec::new();
            for b in batches {
                let col = b.column_by_name("id").ok_or("no id column")?;
                let col = col.as_any().downcast_ref::<UInt64Array>().ok_or("id is not u64")?;
                ids.extend((0..col.len()).map(|i| col.value(i)));
            }
            Result::Ok(ids)
        })?;
        Ok(ids)
    }

    fn delete(&mut self, ids: &[u64]) -> Result<()> {
        // The runner deletes every id divisible by 100; say so directly
        // rather than with a 10,000-item IN list.
        if !ids.iter().all(|id| id % 100 == 0) {
            return Err("this engine only implements the runner's delete pattern".into());
        }
        let table = self.table().clone();
        self.rt.block_on(async move { table.delete("id % 100 = 0").await })?;
        Ok(())
    }

    fn compact(&mut self) -> Result<bool> {
        let table = self.table().clone();
        self.rt.block_on(async move {
            table.optimize(OptimizeAction::All).await?;
            table
                .optimize(OptimizeAction::Prune {
                    older_than: Some(lancedb::table::Duration::try_seconds(0).expect("zero")),
                    delete_unverified: Some(true),
                    error_if_tagged_old_versions: None,
                })
                .await
        })?;
        Ok(true)
    }

    fn disk_bytes(&self) -> u64 {
        vbench::dir_bytes(&self.dir)
    }

    fn describe(&self) -> Value {
        let variant = match self.variant {
            Variant::IvfPq => "IVF-PQ (default partitions and sub-vectors) + refine",
            Variant::HnswSq => "IVF-HNSW-SQ (default partitions)",
        };
        json!({
            "name": "lancedb",
            "label": self.label,
            "variant": variant,
            "metric": "cosine",
            "filter": "bitmap index on root, prefilter (only_if)",
            "k": K,
        })
    }

    fn crash_insert(&mut self, data: &Data) -> Result<()> {
        self.dim = data.dim;
        let n = data.n();
        self.create(batch(data, 0..1000.min(n))?)?;
        vbench::committed(1000.min(n));
        for start in (1000..n).step_by(1000) {
            let end = (start + 1000).min(n);
            let b = batch(data, start..end)?;
            let table = self.table().clone();
            self.rt.block_on(async move { table.add(b).execute().await })?;
            vbench::committed(end);
        }
        Ok(())
    }

    fn crash_verify(&mut self, data: &Data) -> Result<Value> {
        self.dim = data.dim;
        if let Err(e) = self.open() {
            return Ok(json!({ "usable": false, "error": e.to_string() }));
        }
        let table = self.table().clone();
        let rows = self.rt.block_on(async move { table.count_rows(None).await })?;
        self.setting = if self.variant == Variant::IvfPq { 3 } else { 80 };
        let answer = self.search(data.query(0), K, None)?;
        Ok(json!({ "rows": rows, "searchReturned": answer.len(), "usable": true }))
    }
}

fn make(dir: &Path, variant: Option<&str>) -> Result<Box<dyn Engine>> {
    let label = variant.unwrap_or("ivfpq").to_string();
    let variant = match label.as_str() {
        "ivfpq" => Variant::IvfPq,
        "hnswsq" => Variant::HnswSq,
        other => return Err(format!("unknown variant {other} (ivfpq, hnswsq)").into()),
    };
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    Ok(Box::new(Lance { dir: dir.to_path_buf(), variant, label, rt, table: None, setting: 0, dim: 0 }))
}

fn main() {
    vbench::main_with(make);
}
