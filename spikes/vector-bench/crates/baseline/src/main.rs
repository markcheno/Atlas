//! Baseline "engine": exact brute force over a flat file of vectors. It is
//! the reference for binary size and build time (it links what the app
//! already links), and a check on the ground truth: its recall must be 1.0.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vbench::{Data, Engine, K, Knob, Result};

struct Flat {
    dir: PathBuf,
    dim: usize,
    vectors: Vec<f32>,
    roots: Vec<u16>,
    deleted: Vec<bool>,
}

impl Flat {
    fn save(&self) -> Result<()> {
        let bytes: Vec<u8> = self.vectors.iter().flat_map(|x| x.to_le_bytes()).collect();
        fs::write(self.dir.join("vectors.f32"), bytes)?;
        fs::write(self.dir.join("roots.u16"), self.roots.iter().flat_map(|r| r.to_le_bytes()).collect::<Vec<u8>>())?;
        fs::write(self.dir.join("deleted.bin"), self.deleted.iter().map(|d| *d as u8).collect::<Vec<u8>>())?;
        Ok(())
    }
}

impl Engine for Flat {
    fn build(&mut self, data: &Data) -> Result<()> {
        self.dim = data.dim;
        self.vectors = data.base.clone();
        self.roots = data.roots.clone();
        self.deleted = vec![false; data.n()];
        self.save()
    }

    fn reopen(&mut self) -> Result<()> {
        self.vectors = fs::read(self.dir.join("vectors.f32"))?
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        self.roots = fs::read(self.dir.join("roots.u16"))?
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        self.deleted = fs::read(self.dir.join("deleted.bin"))?.into_iter().map(|b| b != 0).collect();
        Ok(())
    }

    fn knobs(&self) -> Vec<Knob> {
        Vec::new()
    }

    fn set_knob(&mut self, _: &Knob) -> Result<()> {
        Ok(())
    }

    fn search(&mut self, q: &[f32], k: usize, root: Option<u16>) -> Result<Vec<u64>> {
        let mut best: Vec<(f32, u64)> = Vec::with_capacity(k + 1);
        for (id, v) in self.vectors.chunks_exact(self.dim).enumerate() {
            if self.deleted[id] || root.is_some_and(|r| self.roots[id] != r) {
                continue;
            }
            let s: f32 = q.iter().zip(v).map(|(a, b)| a * b).sum();
            if best.len() < k || s > best[best.len() - 1].0 {
                let at = best.partition_point(|p| p.0 >= s);
                best.insert(at, (s, id as u64));
                best.truncate(k);
            }
        }
        Ok(best.into_iter().map(|p| p.1).collect())
    }

    fn delete(&mut self, ids: &[u64]) -> Result<()> {
        for id in ids {
            self.deleted[*id as usize] = true;
        }
        self.save()
    }

    fn disk_bytes(&self) -> u64 {
        vbench::dir_bytes(&self.dir)
    }

    fn describe(&self) -> Value {
        json!({ "name": "baseline", "variant": "exact brute force (flat file)", "k": K })
    }

    fn crash_insert(&mut self, data: &Data) -> Result<()> {
        // Appends batches to a file; each flushed batch counts as durable.
        let mut f = fs::File::create(self.dir.join("vectors.f32"))?;
        for (i, batch) in data.base.chunks(1000 * data.dim).enumerate() {
            let bytes: Vec<u8> = batch.iter().flat_map(|x| x.to_le_bytes()).collect();
            f.write_all(&bytes)?;
            f.sync_data()?;
            vbench::committed((i + 1) * 1000);
        }
        Ok(())
    }

    fn crash_verify(&mut self, data: &Data) -> Result<Value> {
        let len = fs::metadata(self.dir.join("vectors.f32"))?.len() as usize;
        let row = data.dim * 4;
        Ok(json!({ "rows": len / row, "tornBytes": len % row, "usable": true }))
    }
}

fn make(dir: &Path, _variant: Option<&str>) -> Result<Box<dyn Engine>> {
    Ok(Box::new(Flat { dir: dir.to_path_buf(), dim: 0, vectors: Vec::new(), roots: Vec::new(), deleted: Vec::new() }))
}

fn main() {
    vbench::main_with(make);
}
