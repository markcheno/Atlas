//! usearch 2.x: an in-memory HNSW graph saved to one file. Variants set the
//! stored scalar type (`f16` by default, `i8`, `f32`); the knob is the
//! search expansion (`ef`). Filtering uses `filtered_search` with a key
//! predicate over a side table of roots (`roots.u16`, saved with the index).
//!
//! Durability is the caller's job: `save` rewrites the whole file, `remove`
//! leaves tombstones that only a rebuild reclaims, and `view` memory-maps a
//! saved file read-only.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;

use serde_json::{Value, json};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};
use vbench::{Data, Engine, K, Knob, Result, knob};

const CONNECTIVITY: usize = 16;
const EXPANSION_ADD: usize = 128;

struct USearch {
    dir: PathBuf,
    options: IndexOptions,
    label: String,
    index: Option<Index>,
    roots: Vec<u16>,
    view_open_ms: Option<f64>,
    threads: usize,
}

impl USearch {
    fn file(&self) -> String {
        self.dir
            .join("index.usearch")
            .to_string_lossy()
            .into_owned()
    }

    fn index(&self) -> &Index {
        self.index.as_ref().expect("index open")
    }

    fn save(&self) -> Result<()> {
        self.index().save(&self.file())?;
        fs::write(
            self.dir.join("roots.u16"),
            self.roots
                .iter()
                .flat_map(|r| r.to_le_bytes())
                .collect::<Vec<u8>>(),
        )?;
        Ok(())
    }

    fn load_roots(&mut self) -> Result<()> {
        self.roots = fs::read(self.dir.join("roots.u16"))?
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        Ok(())
    }

    fn add_all(&self, data: &Data, ids: std::ops::Range<usize>) -> Result<()> {
        let index = self.index();
        let ids: Vec<usize> = ids.collect();
        let per = ids.len().div_ceil(self.threads).max(1);
        thread::scope(|s| -> Result<()> {
            let handles: Vec<_> = ids
                .chunks(per)
                .map(|mine| {
                    s.spawn(move || -> std::result::Result<(), String> {
                        for &id in mine {
                            index
                                .add(id as u64, data.vector(id))
                                .map_err(|e| e.to_string())?;
                        }
                        Ok(())
                    })
                })
                .collect();
            for h in handles {
                h.join().map_err(|_| "insert thread panicked")??;
            }
            Ok(())
        })
    }
}

impl Engine for USearch {
    fn build(&mut self, data: &Data) -> Result<()> {
        self.options.dimensions = data.dim;
        let index = Index::new(&self.options)?;
        index.reserve_capacity_and_threads(data.n(), self.threads)?;
        self.index = Some(index);
        self.roots = data.roots.clone();
        self.add_all(data, 0..data.n())?;
        self.save()
    }

    fn reopen(&mut self) -> Result<()> {
        self.index = None;
        // Memory-mapped (read-only) open, timed for the report.
        let t = Instant::now();
        let viewed = Index::new(&self.options)?;
        viewed.view(&self.file())?;
        self.view_open_ms = Some(vbench::ms(t.elapsed()));
        drop(viewed);
        // Full load, which the rest of the run uses (deletes need it).
        let index = Index::new(&self.options)?;
        index.load(&self.file())?;
        self.index = Some(index);
        self.load_roots()
    }

    fn knobs(&self) -> Vec<Knob> {
        [16.0, 32.0, 64.0, 128.0, 256.0, 512.0]
            .iter()
            .map(|v| knob("ef", *v))
            .collect()
    }

    fn set_knob(&mut self, k: &Knob) -> Result<()> {
        self.index().change_expansion_search(k.value as usize);
        Ok(())
    }

    fn search(&mut self, q: &[f32], k: usize, root: Option<u16>) -> Result<Vec<u64>> {
        let index = self.index();
        let found = match root {
            None => index.search(q, k)?,
            Some(r) => {
                let roots = &self.roots;
                index.filtered_search(q, k, |key| roots.get(key as usize) == Some(&r))?
            }
        };
        Ok(found.keys)
    }

    fn delete(&mut self, ids: &[u64]) -> Result<()> {
        for id in ids {
            self.index().remove(*id)?;
        }
        // Nothing is durable until the whole file is written again.
        self.save()
    }

    fn disk_bytes(&self) -> u64 {
        vbench::dir_bytes(&self.dir)
    }

    fn describe(&self) -> Value {
        json!({
            "name": "usearch",
            "label": self.label,
            "variant": format!("HNSW, {} storage, M={CONNECTIVITY}, ef_construction={EXPANSION_ADD}", self.label),
            "metric": "cosine",
            "filter": "filtered_search with a key predicate (roots side table)",
            "hardware": usearch::hardware_acceleration_compiled(),
            "available": usearch::hardware_acceleration_available(),
            "viewOpenMs": self.view_open_ms,
            "buildThreads": self.threads,
            "memoryUsageBytes": self.index.as_ref().map(|i| i.memory_usage()),
            "k": K,
        })
    }

    fn crash_insert(&mut self, data: &Data) -> Result<()> {
        self.options.dimensions = data.dim;
        let index = Index::new(&self.options)?;
        index.reserve_capacity_and_threads(data.n(), self.threads)?;
        self.index = Some(index);
        self.roots = data.roots.clone();
        // Durable only at each save; save every 10,000 vectors.
        let n = data.n();
        for start in (0..n).step_by(10_000) {
            let end = (start + 10_000).min(n);
            self.add_all(data, start..end)?;
            self.save()?;
            vbench::committed(end);
        }
        Ok(())
    }

    fn crash_verify(&mut self, data: &Data) -> Result<Value> {
        self.options.dimensions = data.dim;
        let index = Index::new(&self.options)?;
        match index.load(&self.file()) {
            Ok(()) => {
                let rows = index.size();
                self.index = Some(index);
                let answer = self.search(data.query(0), K, None)?;
                Ok(json!({ "rows": rows, "searchReturned": answer.len(), "usable": true }))
            }
            Err(e) => Ok(
                json!({ "usable": false, "error": e.to_string(), "recovery": "rebuild from the catalog" }),
            ),
        }
    }
}

fn make(dir: &Path, variant: Option<&str>) -> Result<Box<dyn Engine>> {
    let label = variant.unwrap_or("f16").to_string();
    let quantization = match label.as_str() {
        "f16" => ScalarKind::F16,
        "f32" => ScalarKind::F32,
        "i8" => ScalarKind::I8,
        other => return Err(format!("unknown variant {other} (f16, f32, i8)").into()),
    };
    let options = IndexOptions {
        dimensions: 0,
        metric: MetricKind::Cos,
        quantization,
        connectivity: CONNECTIVITY,
        expansion_add: EXPANSION_ADD,
        expansion_search: 64,
        multi: false,
    };
    let threads = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    Ok(Box::new(USearch {
        dir: dir.to_path_buf(),
        options,
        label,
        index: None,
        roots: Vec::new(),
        view_open_ms: None,
        threads,
    }))
}

fn main() {
    vbench::main_with(make);
}
