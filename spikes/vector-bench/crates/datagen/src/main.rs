//! Generates the synthetic vector set for the spike, deterministically.
//!
//!   datagen --out <dir> [--n 1000000] [--dim 1024] [--queries 1000] [--seed 9]
//!
//! Vectors are unit-length points around random cluster centres (a Gaussian
//! mixture), a rough stand-in for text embeddings: real embeddings cluster
//! by topic. Each vector belongs to one of 8 "source roots" with skewed
//! sizes, so filtered searches can be tested at about 40%, 5% and 0.5%
//! selectivity. Roots are assigned independently of clusters.
//!
//! The exact top-10 for every query (unfiltered and per filter root) is
//! computed by brute force.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::Instant;

use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{Rng, SeedableRng};
use serde_json::json;
use vbench::{Args, K, Result, write_fvecs, write_ivecs};

const ROOT_WEIGHTS: [f64; 8] = [0.40, 0.25, 0.15, 0.10, 0.05, 0.03, 0.015, 0.005];
const FILTER_ROOTS: [u16; 3] = [0, 4, 7];
const CHUNK: usize = 4096;
const SIGMA: f32 = 0.6;

fn uniform(rng: &mut ChaCha8Rng) -> f64 {
    (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// Standard normal (Box–Muller).
fn normal(rng: &mut ChaCha8Rng) -> f32 {
    let u1 = uniform(rng).max(f64::MIN_POSITIVE);
    let u2 = uniform(rng);
    ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32
}

fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= n);
}

fn rng_for(seed: u64, stream: u64, chunk: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed ^ (stream << 48) ^ chunk.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// `count` points around `centres`, generated in parallel chunks, each with
/// its own seeded RNG so the output doesn't depend on the thread count.
fn points(
    centres: &[f32],
    dim: usize,
    count: usize,
    seed: u64,
    stream: u64,
) -> (Vec<f32>, Vec<u16>) {
    let n_centres = centres.len() / dim;
    let mut out = vec![0f32; count * dim];
    let mut roots = vec![0u16; count];
    let cumulative: Vec<f64> = ROOT_WEIGHTS
        .iter()
        .scan(0.0, |s, w| {
            *s += w;
            Some(*s)
        })
        .collect();
    thread::scope(|s| {
        for (c, (vs, rs)) in out
            .chunks_mut(CHUNK * dim)
            .zip(roots.chunks_mut(CHUNK))
            .enumerate()
        {
            let cumulative = &cumulative;
            s.spawn(move || {
                let mut rng = rng_for(seed, stream, c as u64);
                for (v, r) in vs.chunks_exact_mut(dim).zip(rs.iter_mut()) {
                    let centre = (rng.next_u64() % n_centres as u64) as usize;
                    let base = &centres[centre * dim..(centre + 1) * dim];
                    let scale = SIGMA / (dim as f32).sqrt();
                    for (x, b) in v.iter_mut().zip(base) {
                        *x = b + scale * normal(&mut rng);
                    }
                    normalize(v);
                    let u = uniform(&mut rng);
                    *r = cumulative
                        .iter()
                        .position(|c| u < *c)
                        .unwrap_or(ROOT_WEIGHTS.len() - 1) as u16;
                }
            });
        }
    });
    (out, roots)
}

#[derive(Clone)]
struct Top([(f32, u32); K], usize);

impl Top {
    fn new() -> Top {
        Top([(f32::NEG_INFINITY, u32::MAX); K], 0)
    }
    fn push(&mut self, score: f32, id: u32) {
        if self.1 < K {
            self.0[self.1] = (score, id);
            self.1 += 1;
            self.0[..self.1].sort_by(|a, b| b.0.total_cmp(&a.0));
        } else if score > self.0[K - 1].0 {
            self.0[K - 1] = (score, id);
            self.0.sort_by(|a, b| b.0.total_cmp(&a.0));
        }
    }
    fn ids(&self) -> Vec<u32> {
        self.0[..self.1].iter().map(|p| p.1).collect()
    }
}

/// Exact top-K by dot product, unfiltered and for each filter root. Queries
/// are processed in blocks so each pass over the base set serves 16 of them.
fn ground_truth(base: &[f32], roots: &[u16], queries: &[f32], dim: usize) -> Vec<Vec<Top>> {
    const BLOCK: usize = 16;
    let nq = queries.len() / dim;
    let lists = 1 + FILTER_ROOTS.len();
    let mut result = vec![vec![Top::new(); nq]; lists];
    let blocks: Vec<usize> = (0..nq).step_by(BLOCK).collect();
    let threads = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let per = blocks.len().div_ceil(threads);
    let partial: Vec<Vec<(usize, Vec<Top>)>> = thread::scope(|s| {
        let handles: Vec<_> = blocks
            .chunks(per.max(1))
            .map(|mine| {
                s.spawn(move || {
                    let mut out = Vec::new();
                    for &start in mine {
                        let end = (start + BLOCK).min(nq);
                        let mut tops = vec![Top::new(); (end - start) * lists];
                        for (id, v) in base.chunks_exact(dim).enumerate() {
                            let root = roots[id];
                            for (qi, q) in queries[start * dim..end * dim]
                                .chunks_exact(dim)
                                .enumerate()
                            {
                                let score: f32 = q.iter().zip(v).map(|(a, b)| a * b).sum();
                                tops[qi * lists].push(score, id as u32);
                                if let Some(f) = FILTER_ROOTS.iter().position(|r| *r == root) {
                                    tops[qi * lists + 1 + f].push(score, id as u32);
                                }
                            }
                        }
                        out.push((start, tops));
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("ground truth thread"))
            .collect()
    });
    for (start, tops) in partial.into_iter().flatten() {
        for (offset, chunk) in tops.chunks(lists).enumerate() {
            for (list, top) in chunk.iter().enumerate() {
                result[list][start + offset] = top.clone();
            }
        }
    }
    result
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::from_env();
    let out = args.path("out")?;
    let n = args.num("n", 1_000_000)?;
    let dim = args.num("dim", 1024)?;
    let nq = args.num("queries", 1000)?;
    let seed = args.num("seed", 9)? as u64;
    let n_centres = (n / 1000).clamp(64, 2000);
    fs::create_dir_all(&out)?;

    let t = Instant::now();
    let mut rng = rng_for(seed, 0, 0);
    let mut centres = vec![0f32; n_centres * dim];
    for c in centres.chunks_exact_mut(dim) {
        c.iter_mut().for_each(|x| *x = normal(&mut rng));
        normalize(c);
    }
    let (base, roots) = points(&centres, dim, n, seed, 1);
    let (queries, _) = points(&centres, dim, nq, seed, 2);
    eprintln!(
        "datagen: {n} × {dim} generated in {:.1} s",
        t.elapsed().as_secs_f64()
    );

    let t = Instant::now();
    write_fvecs(&out.join("base.fvecs"), dim, &base)?;
    write_fvecs(&out.join("queries.fvecs"), dim, &queries)?;
    fs::write(
        out.join("roots.u16"),
        roots
            .iter()
            .flat_map(|r| r.to_le_bytes())
            .collect::<Vec<u8>>(),
    )?;
    eprintln!("datagen: written in {:.1} s", t.elapsed().as_secs_f64());

    let t = Instant::now();
    let gt = ground_truth(&base, &roots, &queries, dim);
    write_ivecs(
        &out.join("gt.ivecs"),
        &gt[0].iter().map(Top::ids).collect::<Vec<_>>(),
    )?;
    let mut filters = Vec::new();
    for (i, root) in FILTER_ROOTS.iter().enumerate() {
        write_ivecs(
            &out.join(format!("gt-root{root}.ivecs")),
            &gt[1 + i].iter().map(Top::ids).collect::<Vec<_>>(),
        )?;
        let count = roots.iter().filter(|r| *r == root).count();
        filters
            .push(json!({ "root": root, "count": count, "selectivity": count as f64 / n as f64 }));
    }
    eprintln!(
        "datagen: exact top-{K} for {nq} queries in {:.1} s",
        t.elapsed().as_secs_f64()
    );

    let meta = json!({
        "n": n, "dim": dim, "queries": nq, "seed": seed, "centres": n_centres, "sigma": SIGMA,
        "rootWeights": ROOT_WEIGHTS, "filters": filters, "metric": "cosine (unit vectors, dot product)",
    });
    fs::write(out.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
    println!("{meta}");
    let _ = Path::new(&out);
    Ok(())
}
