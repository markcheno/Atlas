//! The map: a place on a plane for each book, so that books near in
//! meaning are near on the page.
//!
//! It starts from the two widest directions of the vectors (PCA, by power
//! iteration), which puts the large shapes in the right places, then pulls
//! each book toward its nearest books and pushes it from books picked at
//! random, as UMAP does. One thread and a seeded generator: the same seed
//! gives the same map on one machine. Coordinates differ in the last bits
//! between machines (`powf`), so nothing may depend on exact values.

use super::knn::Neighbours;
use super::rng::Rng;
use super::{Points, dot};

/// UMAP's curve for a minimum distance of 0.1.
const A: f32 = 1.577;
const B: f32 = 0.895;
/// The largest step one pull or push may take.
const CLIP: f32 = 4.0;
const POWER_ROUNDS: usize = 60;
/// How wide the starting map is.
const START_SPAN: f32 = 10.0;

#[derive(Debug, Clone)]
pub struct LayoutOptions {
    pub seed: u64,
    pub epochs: usize,
    /// Random books pushed away for each pull.
    pub negatives: usize,
}

impl LayoutOptions {
    /// UMAP's own choice: more passes for a small map.
    pub fn for_count(n: usize, seed: u64) -> Self {
        Self {
            seed,
            epochs: if n <= 10_000 { 500 } else { 200 },
            negatives: 5,
        }
    }
}

/// The points along their two widest directions, scaled to the starting
/// span.
pub fn pca2(points: &Points, seed: u64) -> Vec<[f32; 2]> {
    let (n, dims) = (points.len(), points.dims);
    if n == 0 {
        return Vec::new();
    }
    let mut mean = vec![0f32; dims];
    for i in 0..n {
        for (m, x) in mean.iter_mut().zip(points.row(i)) {
            *m += x;
        }
    }
    for m in &mut mean {
        *m /= n as f32;
    }
    let mut rng = Rng::new(seed);
    let mut axes: Vec<Vec<f32>> = Vec::new();
    for _ in 0..2 {
        let mut v: Vec<f32> = (0..dims).map(|_| rng.signed()).collect();
        unit(&mut v);
        for _ in 0..POWER_ROUNDS {
            // v ← Xᵀ(Xv) over the centred rows, without forming them.
            let mean_v = dot(&mean, &v);
            let mut next = vec![0f32; dims];
            let mut total = 0f32;
            for i in 0..n {
                let row = points.row(i);
                let along = dot(row, &v) - mean_v;
                total += along;
                for (x, r) in next.iter_mut().zip(row) {
                    *x += along * r;
                }
            }
            for (x, m) in next.iter_mut().zip(&mean) {
                *x -= total * m;
            }
            for axis in &axes {
                let along = dot(&next, axis);
                for (x, a) in next.iter_mut().zip(axis) {
                    *x -= along * a;
                }
            }
            if !unit(&mut next) {
                break;
            }
            v = next;
        }
        axes.push(v);
    }
    let mut xy: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let row = points.row(i);
            [
                dot(row, &axes[0]) - dot(&mean, &axes[0]),
                dot(row, &axes[1]) - dot(&mean, &axes[1]),
            ]
        })
        .collect();
    let widest = xy
        .iter()
        .flat_map(|p| [p[0].abs(), p[1].abs()])
        .fold(0f32, f32::max);
    if widest > 0.0 {
        for p in &mut xy {
            p[0] *= START_SPAN / widest;
            p[1] *= START_SPAN / widest;
        }
    }
    xy
}

fn unit(v: &mut [f32]) -> bool {
    let len = dot(v, v).sqrt();
    if len > 1e-12 && len.is_finite() {
        for x in v.iter_mut() {
            *x /= len;
        }
        true
    } else {
        false
    }
}

/// One edge of the neighbour graph, with how often it pulls.
struct Edge {
    a: u32,
    b: u32,
    /// Epochs between two pulls of this edge.
    every: f32,
    next: f32,
}

/// The neighbour graph as weighted edges, each pair once. A neighbour's
/// weight falls with its distance beyond the nearest neighbour's, on the
/// scale of that point's own neighbourhood; the two directions of a pair
/// join as UMAP joins them.
fn edges(graph: &Neighbours) -> Vec<Edge> {
    let n = graph.len();
    let mut weights: Vec<(u32, u32, f32)> = Vec::with_capacity(n * graph.k);
    for i in 0..n {
        let sims = graph.sims_of(i);
        let nearest = 1.0 - sims[0];
        let scale = sims.iter().map(|s| (1.0 - s) - nearest).sum::<f32>() / graph.k as f32;
        for (&j, &sim) in graph.ids_of(i).iter().zip(sims) {
            let beyond = (1.0 - sim) - nearest;
            let w = if scale > 1e-9 {
                (-beyond / scale).exp()
            } else {
                1.0
            };
            let (a, b) = if (i as u32) < j {
                (i as u32, j)
            } else {
                (j, i as u32)
            };
            weights.push((a, b, w));
        }
    }
    weights.sort_by_key(|x| (x.0, x.1));
    let mut joined: Vec<(u32, u32, f32)> = Vec::with_capacity(weights.len());
    for (a, b, w) in weights {
        match joined.last_mut() {
            Some(last) if last.0 == a && last.1 == b => last.2 = last.2 + w - last.2 * w,
            _ => joined.push((a, b, w)),
        }
    }
    let strongest = joined.iter().map(|e| e.2).fold(0f32, f32::max);
    joined
        .into_iter()
        .filter(|e| e.2 > 0.0)
        .map(|(a, b, w)| {
            let every = strongest / w;
            Edge {
                a,
                b,
                every,
                next: every,
            }
        })
        .collect()
}

/// The map of `points` whose neighbours are `graph`.
pub fn layout(points: &Points, graph: &Neighbours, opts: &LayoutOptions) -> Vec<[f32; 2]> {
    let n = points.len();
    let mut xy = pca2(points, opts.seed);
    if n < 3 || graph.is_empty() {
        return xy;
    }
    let mut rng = Rng::new(opts.seed ^ 0x5EED);
    // Points in one place can never be pulled apart: a small nudge.
    for p in &mut xy {
        p[0] += rng.signed() * 1e-3;
        p[1] += rng.signed() * 1e-3;
    }
    let mut edges = edges(graph);
    let epochs = opts.epochs.max(1);
    for epoch in 0..epochs {
        let rate = 1.0 - epoch as f32 / epochs as f32;
        let now = (epoch + 1) as f32;
        for e in &mut edges {
            if e.next > now {
                continue;
            }
            e.next += e.every;
            let (a, b) = (e.a as usize, e.b as usize);
            let (dx, dy) = (xy[a][0] - xy[b][0], xy[a][1] - xy[b][1]);
            let d2 = dx * dx + dy * dy;
            if d2 > 0.0 {
                let pull = -2.0 * A * B * d2.powf(B - 1.0) / (1.0 + A * d2.powf(B));
                let (sx, sy) = (step(pull * dx) * rate, step(pull * dy) * rate);
                xy[a][0] += sx;
                xy[a][1] += sy;
                xy[b][0] -= sx;
                xy[b][1] -= sy;
            }
            for _ in 0..opts.negatives {
                let other = rng.below(n);
                if other == a || other == b {
                    continue;
                }
                let (dx, dy) = (xy[a][0] - xy[other][0], xy[a][1] - xy[other][1]);
                let d2 = dx * dx + dy * dy;
                let (sx, sy) = if d2 > 0.0 {
                    let push = 2.0 * B / ((0.001 + d2) * (1.0 + A * d2.powf(B)));
                    (step(push * dx), step(push * dy))
                } else {
                    (CLIP, CLIP)
                };
                xy[a][0] += sx * rate;
                xy[a][1] += sy * rate;
            }
        }
    }
    xy
}

fn step(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(-CLIP, CLIP)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::explore::knn::exact;
    use crate::services::explore::quality::{area_agreement, trustworthiness};
    use crate::services::explore::synthetic::planted;

    #[test]
    fn pca_finds_the_two_widest_directions() {
        // Wide along the 3rd axis, less along the 1st, barely elsewhere.
        let mut rng = Rng::new(4);
        let mut points = Points::new(6);
        let mut wide = Vec::new();
        for _ in 0..300 {
            let (a, b) = (rng.signed() * 5.0, rng.signed() * 2.0);
            wide.push((a, b));
            let row: Vec<f32> = vec![b, 0.1 * rng.signed(), a, 0.1 * rng.signed(), 0.0, 0.05];
            points.push(&row);
        }
        let xy = pca2(&points, 1);
        // The first coordinate follows the widest direction, the second
        // the next (up to sign).
        let corr = |f: &dyn Fn(usize) -> (f32, f32)| {
            let (mut sxy, mut sxx, mut syy) = (0f32, 0f32, 0f32);
            for i in 0..300 {
                let (x, y) = f(i);
                sxy += x * y;
                sxx += x * x;
                syy += y * y;
            }
            (sxy / (sxx * syy).sqrt()).abs()
        };
        assert!(corr(&|i| (xy[i][0], wide[i].0)) > 0.99);
        assert!(corr(&|i| (xy[i][1], wide[i].1)) > 0.99);
        let span = xy.iter().map(|p| p[0].abs()).fold(0f32, f32::max);
        assert!((span - START_SPAN).abs() < 1e-3);
    }

    #[test]
    fn planted_clusters_stay_together_on_the_map() {
        let points = planted(6, 50, 48, 0.6, 21);
        let graph = exact(&points, 15, 2);
        let xy = layout(&points, &graph, &LayoutOptions::for_count(300, 1));
        let groups: Vec<u32> = (0..300).map(|i| i / 50).collect();
        let trust = trustworthiness(&points, &xy, 15, None);
        let agree = area_agreement(&xy, &groups, 10);
        assert!(trust >= 0.90, "trustworthiness {trust}");
        assert!(agree >= 0.90, "agreement {agree}");
    }

    #[test]
    fn the_same_seed_gives_the_same_layout() {
        let points = planted(3, 30, 16, 0.7, 2);
        let graph = exact(&points, 10, 1);
        let opts = LayoutOptions {
            seed: 9,
            epochs: 60,
            negatives: 5,
        };
        let first = layout(&points, &graph, &opts);
        assert_eq!(first, layout(&points, &graph, &opts));
        let other = layout(&points, &graph, &LayoutOptions { seed: 10, ..opts });
        assert_ne!(first, other);
    }

    #[test]
    fn coincident_points_stay_finite() {
        let mut rows = vec![vec![1.0, 0.0, 0.0]; 20];
        rows.extend(vec![vec![0.0, 1.0, 0.0]; 20]);
        let points = Points::from_rows(3, &rows);
        let graph = exact(&points, 5, 1);
        let xy = layout(
            &points,
            &graph,
            &LayoutOptions {
                seed: 1,
                epochs: 100,
                negatives: 5,
            },
        );
        assert_eq!(xy.len(), 40);
        assert!(xy.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
    }
}
