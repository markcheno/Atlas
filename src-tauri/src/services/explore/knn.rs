//! Each book's nearest books, exactly: every pair is compared. At 30,000
//! books that is 450 million dot products, a few seconds on a few
//! threads. The result doesn't depend on how many threads ran.

use super::{Points, dot};

#[derive(Debug, Clone, PartialEq)]
pub struct Neighbours {
    /// Neighbours per point (fewer than asked when there are few points).
    pub k: usize,
    /// `k` row numbers per point, nearest first; equal similarities by
    /// row number.
    pub ids: Vec<u32>,
    pub sims: Vec<f32>,
}

impl Neighbours {
    pub fn len(&self) -> usize {
        self.ids.len().checked_div(self.k).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn ids_of(&self, i: usize) -> &[u32] {
        &self.ids[i * self.k..(i + 1) * self.k]
    }

    pub fn sims_of(&self, i: usize) -> &[f32] {
        &self.sims[i * self.k..(i + 1) * self.k]
    }
}

/// The `k` nearest of each point by cosine, on up to `threads` threads.
pub fn exact(points: &Points, k: usize, threads: usize) -> Neighbours {
    let n = points.len();
    let k = k.min(n.saturating_sub(1));
    if k == 0 {
        return Neighbours {
            k: 0,
            ids: Vec::new(),
            sims: Vec::new(),
        };
    }
    let threads = threads.clamp(1, n);
    let per = n.div_ceil(threads);
    let mut ids = vec![0u32; n * k];
    let mut sims = vec![0f32; n * k];
    std::thread::scope(|scope| {
        for (t, (ids, sims)) in ids
            .chunks_mut(per * k)
            .zip(sims.chunks_mut(per * k))
            .enumerate()
        {
            scope.spawn(move || {
                let first = t * per;
                let mut best: Vec<(f32, u32)> = Vec::with_capacity(k + 1);
                for (row, (ids, sims)) in ids.chunks_mut(k).zip(sims.chunks_mut(k)).enumerate() {
                    let i = first + row;
                    nearest(points, i, k, &mut best);
                    for (slot, (sim, id)) in best.iter().enumerate() {
                        ids[slot] = *id;
                        sims[slot] = *sim;
                    }
                }
            });
        }
    });
    Neighbours { k, ids, sims }
}

/// Fills `best` with the `k` points most like point `i`, nearest first.
fn nearest(points: &Points, i: usize, k: usize, best: &mut Vec<(f32, u32)>) {
    best.clear();
    let own = points.row(i);
    for j in 0..points.len() {
        if j == i {
            continue;
        }
        let sim = dot(own, points.row(j));
        if best.len() == k && sim <= best[k - 1].0 {
            continue;
        }
        // After everything at least as similar: `j` only grows, so equal
        // similarities stay in row order.
        let at = best.partition_point(|(s, _)| *s >= sim);
        best.insert(at, (sim, j as u32));
        best.truncate(k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::explore::synthetic::planted;

    fn brute(points: &Points, i: usize, k: usize) -> Vec<u32> {
        let mut all: Vec<(f32, u32)> = (0..points.len())
            .filter(|&j| j != i)
            .map(|j| (dot(points.row(i), points.row(j)), j as u32))
            .collect();
        all.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        all.into_iter().take(k).map(|(_, j)| j).collect()
    }

    #[test]
    fn neighbours_match_brute_force_whatever_the_threads() {
        let points = planted(4, 25, 24, 0.8, 9);
        let one = exact(&points, 7, 1);
        assert_eq!(one.len(), 100);
        for i in 0..points.len() {
            assert_eq!(one.ids_of(i), brute(&points, i, 7), "point {i}");
            assert!(one.sims_of(i).windows(2).all(|w| w[0] >= w[1]));
        }
        for threads in [2, 3, 8, 500] {
            assert_eq!(exact(&points, 7, threads), one, "{threads} threads");
        }
    }

    #[test]
    fn ties_break_by_index() {
        // Four identical points and one apart.
        let rows = vec![
            vec![1.0, 0.0],
            vec![1.0, 0.0],
            vec![0.0, 1.0],
            vec![1.0, 0.0],
            vec![1.0, 0.0],
        ];
        let points = Points::from_rows(2, &rows);
        let found = exact(&points, 3, 2);
        assert_eq!(found.ids_of(0), [1, 3, 4]);
        assert_eq!(found.ids_of(4), [0, 1, 3]);
        assert_eq!(found.ids_of(2), [0, 1, 3]);
    }

    #[test]
    fn few_points_have_fewer_neighbours() {
        let points = Points::from_rows(2, &[vec![1.0, 0.0], vec![0.0, 1.0]]);
        let found = exact(&points, 15, 4);
        assert_eq!(found.k, 1);
        assert_eq!(found.ids, [1, 0]);
        assert!(exact(&Points::from_rows(2, &[vec![1.0, 0.0]]), 15, 4).is_empty());
    }
}
