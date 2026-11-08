// sqlite-vec (`vec0` virtual tables), statically linked into the same
// bundled SQLite the app uses. Shared by vec-sqlite (published crate),
// vec-sqlite-simd (same C source with NEON/AVX) and vec-sqlite-alpha
// (0.1.10 alpha, DiskANN). Variants:
//
// - `f32`, `f32-l2`: exact search over float vectors, cosine or L2. On unit
//   vectors both rank identically; only L2 has SIMD code in sqlite-vec.
// - `int8`, `int8-l2`: vectors quantized to int8 with one scale taken from
//   the data's largest component (stored with the index), brute force.
//   (sqlite-vec's own `vec_quantize_int8(v, 'unit')` maps ±1 to ±127, which
//   leaves only a few levels for the ±0.03 components of a 1024-d unit
//   vector.)
// - `bit`: binary-quantized candidates (Hamming), then rescored exactly
//   against the float vectors; the knob is how many candidates to rescore.
// - `diskann` (alpha build only): graph index with binary-quantized
//   neighbours. It can't be combined with a partition key, so filtered
//   searches fetch extra candidates and filter by root afterwards.
//
// Otherwise the source root is a `partition key`, so a filtered search
// scans only that folder's vectors.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use vbench::{Data, Engine, K, Knob, Result, knob};

#[derive(Clone, Copy, PartialEq)]
enum Variant {
    F32,
    Int8,
    Bit,
    DiskAnn,
}

const DISKANN_FILTER_FETCH: usize = 400;

struct SqliteVec {
    path: PathBuf,
    variant: Variant,
    dim: usize,
    conn: Option<Connection>,
    rescore: usize,
    l2: bool,
    label: String,
    /// DiskANN only: root of each id, loaded from the `roots` table.
    roots: Vec<u16>,
    /// int8 only: multiply by this, round and clamp to get the int8 value.
    scale: f32,
}

fn int8_blob(v: &[f32], scale: f32) -> Vec<u8> {
    v.iter().map(|x| (x * scale).round().clamp(-127.0, 127.0) as i8 as u8).collect()
}

fn blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn register() {
    // SAFETY: registers sqlite-vec's entry point for every new connection;
    // the signature matches what SQLite expects of an extension init.
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<*const (), unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut std::os::raw::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> i32>(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }
}

impl SqliteVec {
    fn open(&mut self) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.pragma_update(None, "synchronous", "normal")?;
        self.conn = Some(conn);
        Ok(())
    }

    fn conn(&self) -> &Connection {
        self.conn.as_ref().expect("open")
    }

    fn create(&self) -> Result<()> {
        let d = self.dim;
        let metric = if self.l2 { "l2" } else { "cosine" };
        let sql = match self.variant {
            Variant::F32 => format!(
                "CREATE VIRTUAL TABLE v USING vec0(root integer partition key, embedding float[{d}] distance_metric={metric})"
            ),
            Variant::Int8 => format!(
                "CREATE VIRTUAL TABLE v USING vec0(root integer partition key, embedding int8[{d}] distance_metric={metric});
                 CREATE TABLE settings(scale REAL NOT NULL);
                 INSERT INTO settings VALUES ({})",
                self.scale
            ),
            Variant::Bit => format!(
                "CREATE VIRTUAL TABLE v USING vec0(root integer partition key, embedding bit[{d}]);
                 CREATE TABLE full(id INTEGER PRIMARY KEY, embedding BLOB NOT NULL)"
            ),
            Variant::DiskAnn => format!(
                "CREATE VIRTUAL TABLE v USING vec0(embedding float[{d}] distance_metric={metric} INDEXED BY diskann(neighbor_quantizer=binary));
                 CREATE TABLE roots(id INTEGER PRIMARY KEY, root INTEGER NOT NULL)"
            ),
        };
        self.conn().execute_batch(&sql)?;
        Ok(())
    }

    fn insert(&self, data: &Data, range: std::ops::Range<usize>) -> Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        {
            let value = match self.variant {
                Variant::F32 | Variant::DiskAnn => "?3",
                Variant::Int8 => "vec_int8(?3)",
                Variant::Bit => "vec_quantize_binary(?3)",
            };
            let mut ins = if self.variant == Variant::DiskAnn {
                tx.prepare(&format!("INSERT INTO v(rowid, embedding) VALUES (?1, {})", value.replace("?3", "?2")))?
            } else {
                tx.prepare(&format!("INSERT INTO v(rowid, root, embedding) VALUES (?1, ?2, {value})"))?
            };
            let mut side = match self.variant {
                Variant::Bit => Some(tx.prepare("INSERT INTO full(id, embedding) VALUES (?1, ?2)")?),
                Variant::DiskAnn => Some(tx.prepare("INSERT INTO roots(id, root) VALUES (?1, ?2)")?),
                _ => None,
            };
            for id in range {
                let b = if self.variant == Variant::Int8 {
                    int8_blob(data.vector(id), self.scale)
                } else {
                    blob(data.vector(id))
                };
                if self.variant == Variant::DiskAnn {
                    // Rowids start at 1, as in sqlite-vec's own DiskANN tests.
                    ins.execute(params![id as i64 + 1, b])?;
                } else {
                    ins.execute(params![id as i64, data.roots[id] as i64, b])?;
                }
                match (self.variant, side.as_mut()) {
                    (Variant::Bit, Some(st)) => {
                        st.execute(params![id as i64, b])?;
                    }
                    (Variant::DiskAnn, Some(st)) => {
                        st.execute(params![id as i64, data.roots[id] as i64])?;
                    }
                    _ => {}
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
}

impl Engine for SqliteVec {
    fn build(&mut self, data: &Data) -> Result<()> {
        self.dim = data.dim;
        self.scale = 127.0 / data.base.iter().fold(0f32, |m, x| m.max(x.abs()));
        self.open()?;
        self.create()?;
        let n = data.n();
        for start in (0..n).step_by(10_000) {
            self.insert(data, start..(start + 10_000).min(n))?;
        }
        self.conn().execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }

    fn reopen(&mut self) -> Result<()> {
        self.conn = None;
        self.open()?;
        if self.variant == Variant::Int8 {
            self.scale = self.conn().query_row("SELECT scale FROM settings", [], |r| r.get::<_, f64>(0))? as f32;
        }
        if self.variant == Variant::DiskAnn {
            let mut roots = Vec::new();
            {
                let mut st = self.conn().prepare("SELECT id, root FROM roots")?;
                for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
                    let (id, root) = row?;
                    if roots.len() <= id as usize {
                        roots.resize(id as usize + 1, u16::MAX);
                    }
                    roots[id as usize] = root as u16;
                }
            }
            self.roots = roots;
        }
        Ok(())
    }

    fn knobs(&self) -> Vec<Knob> {
        match self.variant {
            Variant::Bit => [10.0, 20.0, 50.0, 100.0, 200.0, 400.0].iter().map(|v| knob("rescore", *v)).collect(),
            Variant::DiskAnn => {
                [32.0, 64.0, 128.0, 256.0, 512.0].iter().map(|v| knob("search_list_size_search", *v)).collect()
            }
            _ => Vec::new(),
        }
    }

    fn set_knob(&mut self, k: &Knob) -> Result<()> {
        self.rescore = k.value as usize;
        if self.variant == Variant::DiskAnn {
            self.conn().execute("INSERT INTO v(v) VALUES (?1)", [format!("search_list_size_search={}", k.value as usize)])?;
        }
        Ok(())
    }

    fn search(&mut self, q: &[f32], k: usize, root: Option<u16>) -> Result<Vec<u64>> {
        let filter = if root.is_some() { "AND root = ?3" } else { "" };
        let b = if self.variant == Variant::Int8 { int8_blob(q, self.scale) } else { blob(q) };
        let conn = self.conn();
        let rows: Vec<i64> = match self.variant {
            Variant::DiskAnn => {
                let fetch = if root.is_some() { DISKANN_FILTER_FETCH } else { k };
                let mut st =
                    conn.prepare_cached("SELECT rowid FROM v WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance")?;
                let all: Vec<i64> = st.query_map(params![b, fetch as i64], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                all.into_iter()
                    .map(|rowid| rowid - 1)
                    .filter(|id| root.is_none_or(|want| self.roots.get(*id as usize) == Some(&want)))
                    .take(k)
                    .collect()
            }
            Variant::F32 | Variant::Int8 => {
                let m = if self.variant == Variant::F32 { "?1" } else { "vec_int8(?1)" };
                let mut st = conn.prepare_cached(&format!(
                    "SELECT rowid FROM v WHERE embedding MATCH {m} AND k = ?2 {filter} ORDER BY distance"
                ))?;
                match root {
                    Some(r) => st.query_map(params![b, k as i64, r as i64], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?,
                    None => st.query_map(params![b, k as i64], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?,
                }
            }
            Variant::Bit => {
                let mut st = conn.prepare_cached(&format!(
                    "WITH c AS (SELECT rowid AS id FROM v WHERE embedding MATCH vec_quantize_binary(?1) AND k = ?2 {filter})
                     SELECT c.id FROM c JOIN full f ON f.id = c.id
                     ORDER BY vec_distance_cosine(f.embedding, ?1) LIMIT {k}"
                ))?;
                let over = self.rescore.max(k) as i64;
                match root {
                    Some(r) => st.query_map(params![b, over, r as i64], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?,
                    None => st.query_map(params![b, over], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?,
                }
            }
        };
        Ok(rows.into_iter().map(|r| r as u64).collect())
    }

    fn delete(&mut self, ids: &[u64]) -> Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        {
            let mut del = tx.prepare("DELETE FROM v WHERE rowid = ?1")?;
            let offset = if self.variant == Variant::DiskAnn { 1 } else { 0 };
            for id in ids {
                del.execute([*id as i64 + offset])?;
            }
            let side = match self.variant {
                Variant::Bit => Some("DELETE FROM full WHERE id = ?1"),
                Variant::DiskAnn => Some("DELETE FROM roots WHERE id = ?1"),
                _ => None,
            };
            if let Some(sql) = side {
                let mut del = tx.prepare(sql)?;
                for id in ids {
                    del.execute([*id as i64])?;
                }
            }
        }
        tx.commit()?;
        self.conn().execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }

    fn compact(&mut self) -> Result<bool> {
        if self.variant == Variant::DiskAnn {
            // The alpha keeps statements of its own open, so VACUUM fails
            // ("SQL statements in progress").
            return Ok(false);
        }
        self.conn().flush_prepared_statement_cache();
        self.conn().execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(true)
    }

    fn disk_bytes(&self) -> u64 {
        let wal = PathBuf::from(format!("{}-wal", self.path.display()));
        vbench::dir_bytes(&self.path) + vbench::dir_bytes(&wal)
    }

    fn describe(&self) -> Value {
        let versions = self
            .conn
            .as_ref()
            .and_then(|c| c.query_row("SELECT sqlite_version(), vec_version()", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).ok());
        let variant = match self.variant {
            Variant::F32 => "f32 brute force",
            Variant::Int8 => "int8 brute force",
            Variant::Bit => "bit candidates + exact f32 rescoring",
            Variant::DiskAnn => "DiskANN (binary neighbours), alpha",
        };
        let simd = simd_label();
        json!({
            "name": format!("sqlite-vec ({BUILD})"),
            "label": self.label,
            "variant": variant,
            "metric": if self.l2 { "l2" } else { "cosine" },
            "int8Scale": if self.variant == Variant::Int8 { Some(self.scale) } else { None },
            "simd": simd,
            "sqlite": versions.as_ref().map(|v| v.0.clone()),
            "sqliteVec": versions.map(|v| v.1),
            "filter": if self.variant == Variant::DiskAnn {
                format!("post-filter after fetching {DISKANN_FILTER_FETCH} (no partition keys with DiskANN)")
            } else {
                "partition key on root".to_string()
            },
            "k": K,
        })
    }

    fn crash_insert(&mut self, data: &Data) -> Result<()> {
        self.dim = data.dim;
        self.scale = 127.0 / data.base.iter().fold(0f32, |m, x| m.max(x.abs()));
        self.open()?;
        self.create()?;
        let n = data.n();
        for start in (0..n).step_by(1000) {
            let end = (start + 1000).min(n);
            self.insert(data, start..end)?;
            vbench::committed(end);
        }
        Ok(())
    }

    fn crash_verify(&mut self, data: &Data) -> Result<Value> {
        self.dim = data.dim;
        self.reopen()?;
        let integrity: String = self.conn().query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        let rows: i64 = self.conn().query_row("SELECT count(*) FROM v", [], |r| r.get(0))?;
        let answer = self.search(data.query(0), K, None)?;
        Ok(json!({ "rows": rows, "integrity": integrity, "searchReturned": answer.len(), "usable": integrity == "ok" }))
    }
}

fn make(dir: &Path, variant: Option<&str>) -> Result<Box<dyn Engine>> {
    register();
    let label = variant.unwrap_or("f32").to_string();
    let (variant, l2) = match label.as_str() {
        "f32" => (Variant::F32, false),
        "f32-l2" => (Variant::F32, true),
        "int8" => (Variant::Int8, false),
        "int8-l2" => (Variant::Int8, true),
        "bit" => (Variant::Bit, false),
        "diskann" if BUILD == "alpha" => (Variant::DiskAnn, false),
        "diskann-l2" if BUILD == "alpha" => (Variant::DiskAnn, true),
        other => return Err(format!("unknown variant {other} for the {BUILD} build").into()),
    };
    Ok(Box::new(SqliteVec { path: dir.join("vectors.sqlite"), variant, dim: 0, conn: None, rescore: 100, l2, label, roots: Vec::new(), scale: 1.0 }))
}

fn main() {
    vbench::main_with(make);
}
