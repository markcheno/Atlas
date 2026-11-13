//! Measures of the areas and of the map, as numbers the owner can share:
//! none of them holds a word from a book.

use super::cluster::Area;
use super::knn::Neighbours;
use super::rng::Rng;
use super::{Points, dot};

/// The `k` nearest points to `i` on the map, nearest first, ties by row.
fn nearest_on_map(xy: &[[f32; 2]], i: usize, k: usize) -> Vec<u32> {
    let mut best: Vec<(f32, u32)> = Vec::with_capacity(k + 1);
    for (j, p) in xy.iter().enumerate() {
        if j == i {
            continue;
        }
        let (dx, dy) = (p[0] - xy[i][0], p[1] - xy[i][1]);
        let d = dx * dx + dy * dy;
        if best.len() == k && d >= best[k - 1].0 {
            continue;
        }
        let at = best.partition_point(|(s, _)| *s <= d);
        best.insert(at, (d, j as u32));
        best.truncate(k);
    }
    best.into_iter().map(|(_, j)| j).collect()
}

/// Trustworthiness (Venna and Kaski): 1 when every point's `k` nearest on
/// the map are among its `k` nearest by meaning; lower the further away
/// the map's intruders really are. `sample` measures that many rows,
/// picked by the seed, instead of all.
pub fn trustworthiness(
    points: &Points,
    xy: &[[f32; 2]],
    k: usize,
    sample: Option<(usize, u64)>,
) -> f32 {
    let n = points.len();
    let k = k.min(n.saturating_sub(1) / 2);
    if k == 0 || 2 * n <= 3 * k + 1 {
        return 1.0;
    }
    let rows: Vec<usize> = match sample {
        Some((count, seed)) if count < n => {
            let mut rng = Rng::new(seed);
            let mut all: Vec<usize> = (0..n).collect();
            for i in 0..count {
                let j = i + rng.below(n - i);
                all.swap(i, j);
            }
            all.truncate(count);
            all
        }
        _ => (0..n).collect(),
    };
    let mut penalty = 0f64;
    let mut sims = vec![0f32; n];
    for &i in &rows {
        for (j, s) in sims.iter_mut().enumerate() {
            *s = dot(points.row(i), points.row(j));
        }
        for j in nearest_on_map(xy, i, k) {
            let j = j as usize;
            // The rank of `j` among `i`'s neighbours by meaning.
            let rank = 1
                + (0..n)
                    .filter(|&l| l != i && l != j)
                    .filter(|&l| sims[l] > sims[j] || (sims[l] == sims[j] && l < j))
                    .count();
            if rank > k {
                penalty += (rank - k) as f64;
            }
        }
    }
    let scale = 2.0 / (rows.len() as f64 * k as f64 * (2.0 * n as f64 - 3.0 * k as f64 - 1.0));
    (1.0 - scale * penalty) as f32
}

/// The share of each point's `k` nearest by meaning that are also among
/// its `k` nearest on the map.
pub fn neighbour_preservation(high: &Neighbours, xy: &[[f32; 2]], k: usize) -> f32 {
    let k = k.min(high.k);
    if k == 0 || high.is_empty() {
        return 1.0;
    }
    let mut kept = 0usize;
    for i in 0..high.len() {
        let on_map = nearest_on_map(xy, i, k);
        kept += high.ids_of(i)[..k]
            .iter()
            .filter(|j| on_map.contains(j))
            .count();
    }
    kept as f32 / (high.len() * k) as f32
}

/// The share of each point's `k` nearest on the map that are in its own
/// area: whether areas show as regions.
pub fn area_agreement(xy: &[[f32; 2]], area_of: &[u32], k: usize) -> f32 {
    let n = xy.len();
    let k = k.min(n.saturating_sub(1));
    if k == 0 {
        return 1.0;
    }
    let mut same = 0usize;
    for i in 0..n {
        same += nearest_on_map(xy, i, k)
            .iter()
            .filter(|&&j| area_of[j as usize] == area_of[i])
            .count();
    }
    same as f32 / (n * k) as f32
}

/// A silhouette by centres: for each point, how much nearer its own
/// area's centre is than the next area's, from -1 to 1, averaged.
pub fn silhouette(points: &Points, areas: &[Area]) -> f32 {
    if areas.len() < 2 || points.is_empty() {
        return 0.0;
    }
    let mut total = 0f32;
    let mut count = 0usize;
    for (own, area) in areas.iter().enumerate() {
        for &m in &area.members {
            let row = points.row(m as usize);
            let a = 1.0 - dot(row, &area.centre);
            let b = areas
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != own)
                .map(|(_, other)| 1.0 - dot(row, &other.centre))
                .fold(f32::INFINITY, f32::min);
            let widest = a.max(b);
            if widest > 0.0 {
                total += (b - a) / widest;
            }
            count += 1;
        }
    }
    total / count.max(1) as f32
}

/// Pairs of points more alike than `floor`, each pair once: likely two
/// editions of one text.
pub fn near_pairs(graph: &Neighbours, floor: f32) -> usize {
    (0..graph.len())
        .map(|i| {
            graph
                .ids_of(i)
                .iter()
                .zip(graph.sims_of(i))
                .filter(|(j, s)| **s >= floor && (i as u32) < **j)
                .count()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::explore::cluster::{AreaOptions, areas};
    use crate::services::explore::knn::exact;
    use crate::services::explore::synthetic::planted;

    /// Points on a plane, as 2-D unit-free vectors padded to 3 dims and
    /// their own map.
    fn plane(n: usize) -> (Points, Vec<[f32; 2]>) {
        let mut rng = Rng::new(3);
        let mut points = Points::new(3);
        let mut xy = Vec::new();
        for _ in 0..n {
            // On the unit sphere near its pole, so the cosine order is
            // the distance order on the plane.
            let (x, y) = (rng.signed() * 0.2, rng.signed() * 0.2);
            let z = (1.0 - x * x - y * y).sqrt();
            points.push(&[x, y, z]);
            xy.push([x, y]);
        }
        (points, xy)
    }

    #[test]
    fn trustworthiness_is_one_for_a_faithful_map_and_low_for_a_shuffle() {
        let (points, xy) = plane(200);
        let faithful = trustworthiness(&points, &xy, 10, None);
        assert!(faithful > 0.999, "{faithful}");
        let graph = exact(&points, 10, 1);
        assert!(neighbour_preservation(&graph, &xy, 10) > 0.99);

        let mut shuffled = xy.clone();
        shuffled.rotate_left(71);
        let random = trustworthiness(&points, &shuffled, 10, None);
        assert!(random < 0.65, "{random}");
        assert!(neighbour_preservation(&graph, &shuffled, 10) < 0.2);
        // A sample of rows tells the same story.
        let sampled = trustworthiness(&points, &xy, 10, Some((50, 1)));
        assert!(sampled > 0.999, "{sampled}");
    }

    #[test]
    fn areas_that_are_regions_agree_and_scattered_ones_do_not() {
        let (_, xy) = plane(200);
        let left: Vec<u32> = xy.iter().map(|p| u32::from(p[0] > 0.0)).collect();
        assert!(area_agreement(&xy, &left, 10) > 0.85);
        let scattered: Vec<u32> = (0..200).map(|i| i % 2).collect();
        assert!(area_agreement(&xy, &scattered, 10) < 0.65);
    }

    #[test]
    fn the_silhouette_is_high_for_planted_areas() {
        let points = planted(4, 40, 32, 0.4, 8);
        let found = areas(
            &points,
            &AreaOptions {
                areas: 4,
                min_area: 5,
                sub_areas: 1,
                min_sub_area: 3,
                seed: 1,
            },
        );
        assert!(silhouette(&points, &found) > 0.5);
        assert_eq!(silhouette(&points, &found[..1]), 0.0);
    }

    #[test]
    fn near_pairs_are_counted_once() {
        let rows = vec![
            vec![1.0, 0.0],
            vec![1.0, 0.0],
            vec![0.0, 1.0],
            vec![0.0, -1.0],
        ];
        let graph = exact(&Points::from_rows(2, &rows), 2, 1);
        assert_eq!(near_pairs(&graph, 0.98), 1);
    }
}
