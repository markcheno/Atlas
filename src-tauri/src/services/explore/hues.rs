//! A colour for each area (Slice 4.6.1), as the spike's page chose them:
//! one of eight hues of the reference chart palette, so that areas side by
//! side on the map never share a hue, nor take a pair too close to tell
//! apart. Kept across rebuilds where an area survives and still can.
//!
//! The hues themselves (light and dark steps) live in the window
//! (`src/lib/exploreColours.ts`); here a hue is its index.

use std::collections::HashMap;

/// How many hues there are.
pub const HUES: usize = 8;

/// Pairs of hues the palette validator finds too close side by side (its
/// colour-blind or normal-vision floor, in either mode), 0-based.
const CLOSE: [(u8, u8); 12] = [
    (0, 6),
    (1, 3),
    (1, 4),
    (1, 5),
    (1, 7),
    (2, 4),
    (2, 5),
    (2, 7),
    (3, 5),
    (3, 7),
    (4, 7),
    (5, 7),
];

fn close(a: u8, b: u8) -> bool {
    let (a, b) = if a < b { (a, b) } else { (b, a) };
    CLOSE.contains(&(a, b))
}

/// How much each pair of areas touches on the map: among each book's six
/// nearest books on the map, those in another area. A grid keeps it near
/// linear in the number of books.
pub fn touching(xy: &[[f32; 2]], area_of: &[u32], areas: usize) -> Vec<Vec<f64>> {
    const K: usize = 6;
    let mut w = vec![vec![0f64; areas]; areas];
    let n = xy.len();
    if n < 2 || areas == 0 {
        return w;
    }
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in xy {
        min_x = min_x.min(p[0]);
        max_x = max_x.max(p[0]);
        min_y = min_y.min(p[1]);
        max_y = max_y.max(p[1]);
    }
    let side = ((n as f64 / 4.0).sqrt().round() as usize).max(1);
    let sx = (max_x - min_x).max(1e-9) / side as f32;
    let sy = (max_y - min_y).max(1e-9) / side as f32;
    let cell = |v: f32, min: f32, s: f32| (((v - min) / s) as usize).min(side - 1);
    let mut grid: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, p) in xy.iter().enumerate() {
        grid.entry((cell(p[0], min_x, sx), cell(p[1], min_y, sy)))
            .or_default()
            .push(i);
    }
    let far = |i: usize, j: usize| {
        let (dx, dy) = (xy[i][0] - xy[j][0], xy[i][1] - xy[j][1]);
        dx * dx + dy * dy
    };
    for i in 0..n {
        let (cx, cy) = (cell(xy[i][0], min_x, sx), cell(xy[i][1], min_y, sy));
        let mut near = Vec::new();
        let mut found = false;
        // Widen until K books are found, then one ring more, so a book at
        // a cell's edge finds its neighbours over it.
        for ring in 1..=side {
            near.clear();
            for gx in cx.saturating_sub(ring)..=(cx + ring).min(side - 1) {
                for gy in cy.saturating_sub(ring)..=(cy + ring).min(side - 1) {
                    if let Some(ids) = grid.get(&(gx, gy)) {
                        near.extend(ids.iter().copied().filter(|&j| j != i));
                    }
                }
            }
            if found {
                break;
            }
            found = near.len() >= K;
        }
        near.sort_by(|&p, &q| far(i, p).total_cmp(&far(i, q)).then(p.cmp(&q)));
        for &j in near.iter().take(K) {
            let (a, b) = (area_of[i] as usize, area_of[j] as usize);
            if a != b && a < areas && b < areas {
                w[a][b] += 1.0;
                w[b][a] += 1.0;
            }
        }
    }
    w
}

/// A hue for each area from how they touch (`w`). An area with a hue to
/// keep keeps it unless an area it touches already has it; the rest are
/// coloured most-touched first, each taking the hue that costs least: a
/// hue a touching area has costs 100 per touch, a pair too close to tell
/// apart 10, and the hue used least wins a tie.
pub fn assign(w: &[Vec<f64>], keep: &[Option<u8>]) -> Vec<u8> {
    let n = w.len();
    let mut hue: Vec<Option<u8>> = vec![None; n];
    let mut used = [0usize; HUES];
    let total: Vec<f64> = w.iter().map(|row| row.iter().sum()).collect();

    // Kept hues first, most-touched first.
    let mut kept: Vec<usize> = (0..n)
        .filter(|&a| keep.get(a).copied().flatten().is_some())
        .collect();
    kept.sort_by(|&a, &b| total[b].total_cmp(&total[a]).then(a.cmp(&b)));
    for a in kept {
        let k = keep[a].unwrap_or(0) % HUES as u8;
        let clash = (0..n).any(|b| w[a][b] > 0.0 && hue[b] == Some(k));
        if !clash {
            hue[a] = Some(k);
            used[k as usize] += 1;
        }
    }

    while let Some(next) = (0..n).filter(|&a| hue[a].is_none()).max_by(|&a, &b| {
        let sat = |x: usize| {
            (0..n)
                .filter(|&y| hue[y].is_some())
                .map(|y| w[x][y])
                .sum::<f64>()
        };
        sat(a)
            .total_cmp(&sat(b))
            .then(total[a].total_cmp(&total[b]))
            .then(b.cmp(&a))
    }) {
        let mut pick = 0u8;
        let mut cost = f64::INFINITY;
        for c in 0..HUES as u8 {
            let mut k = used[c as usize] as f64 * 1e-3;
            for b in 0..n {
                let Some(hb) = hue[b] else { continue };
                if w[next][b] == 0.0 {
                    continue;
                }
                if hb == c {
                    k += 100.0 * w[next][b];
                } else if close(c, hb) {
                    k += 10.0 * w[next][b];
                }
            }
            if k < cost {
                cost = k;
                pick = c;
            }
        }
        hue[next] = Some(pick);
        used[pick as usize] += 1;
    }
    hue.into_iter().map(|h| h.unwrap_or(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ring of areas, each touching the next.
    #[allow(clippy::needless_range_loop)]
    fn ring(n: usize) -> Vec<Vec<f64>> {
        let mut w = vec![vec![0.0; n]; n];
        for a in 0..n {
            let b = (a + 1) % n;
            w[a][b] = 10.0;
            w[b][a] = 10.0;
        }
        w
    }

    fn clashes(w: &[Vec<f64>], hues: &[u8]) -> usize {
        let mut bad = 0;
        for a in 0..w.len() {
            for b in a + 1..w.len() {
                if w[a][b] > 0.0 && (hues[a] == hues[b] || close(hues[a], hues[b])) {
                    bad += 1;
                }
            }
        }
        bad
    }

    #[test]
    fn touching_areas_never_share_a_hue_or_a_close_pair() {
        for n in [2, 5, 8, 16] {
            let w = ring(n);
            let hues = assign(&w, &vec![None; n]);
            assert_eq!(clashes(&w, &hues), 0, "{n} areas: {hues:?}");
        }
    }

    #[test]
    fn kept_hues_stay_unless_they_now_clash() {
        let w = ring(6);
        let first = assign(&w, &[None; 6]);
        let keep: Vec<Option<u8>> = first.iter().map(|h| Some(*h)).collect();
        assert_eq!(assign(&w, &keep), first);
        // Two touching areas asked to keep one hue: the second gives way.
        let mut keep = keep;
        keep[1] = keep[0];
        let again = assign(&w, &keep);
        assert_ne!(again[0], again[1]);
    }

    #[test]
    fn areas_side_by_side_on_the_map_touch() {
        // Two blobs of books, left and right, plus a third far away.
        let mut xy = Vec::new();
        let mut area_of = Vec::new();
        for i in 0..40 {
            let (x, y) = ((i % 8) as f32 * 0.1, (i / 8) as f32 * 0.1);
            xy.push([x, y]);
            area_of.push(0);
            xy.push([x + 0.85, y]);
            area_of.push(1);
            xy.push([x + 50.0, y + 50.0]);
            area_of.push(2);
        }
        let w = touching(&xy, &area_of, 3);
        assert!(w[0][1] > 0.0 && w[1][0] == w[0][1]);
        assert_eq!(w[0][2] + w[1][2], 0.0);
    }
}
