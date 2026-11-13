//! The Explore spike (Slice 4.1.2, docs/spike-explore.md): can the
//! library be laid out by meaning, as named areas on a map?
//!
//! Pure functions over the vectors the catalog already holds: one vector
//! per book, areas by clustering, a 2-D layout, measures of both, and
//! names from the books' own words. Nothing here is called by the app: a
//! harness the owner runs (`tests/explore.rs`, `just spike-explore`) reads
//! a library and writes one page to look at. If the spike says stop, this
//! module is removed.
//!
//! Hand-written, with no new dependency: a few hundred lines of textbook
//! methods, against crates to license, build on three OSes and then
//! delete if the answer is no.

pub mod books;
pub mod cluster;
pub mod knn;
pub mod layout;
pub mod names;
pub mod quality;
pub mod rng;
pub mod survey;
pub mod synthetic;

use crate::services::embedding::vector::normalize;

/// Unit vectors, row after row.
#[derive(Debug, Clone, PartialEq)]
pub struct Points {
    pub dims: usize,
    data: Vec<f32>,
}

impl Points {
    pub fn new(dims: usize) -> Self {
        assert!(dims > 0, "a vector has at least one number");
        Self {
            dims,
            data: Vec::new(),
        }
    }

    pub fn from_rows(dims: usize, rows: &[Vec<f32>]) -> Self {
        let mut points = Self::new(dims);
        for row in rows {
            points.push(row);
        }
        points
    }

    pub fn push(&mut self, row: &[f32]) {
        assert_eq!(row.len(), self.dims, "a row of another length");
        self.data.extend_from_slice(row);
    }

    pub fn len(&self) -> usize {
        self.data.len() / self.dims
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn row(&self, i: usize) -> &[f32] {
        &self.data[i * self.dims..(i + 1) * self.dims]
    }

    /// The same points with what they all share taken out: the mean
    /// direction is subtracted and each row made unit length again.
    /// Averaged book vectors lean one way together, which crowds their
    /// cosines; this is the spike's second fallback. A row equal to the
    /// mean is left as it was.
    pub fn centred(&self) -> Self {
        let n = self.len();
        let mut mean = vec![0f32; self.dims];
        for i in 0..n {
            for (m, x) in mean.iter_mut().zip(self.row(i)) {
                *m += x;
            }
        }
        for m in &mut mean {
            *m /= n.max(1) as f32;
        }
        let mut out = Self::new(self.dims);
        for i in 0..n {
            let mut row: Vec<f32> = self.row(i).iter().zip(&mean).map(|(x, m)| x - m).collect();
            if normalize(&mut row) {
                out.push(&row);
            } else {
                out.push(self.row(i));
            }
        }
        out
    }
}

/// The dot product in eight fixed lanes: several times faster than one
/// running sum, and the same bits on every machine, since the order of
/// the additions never depends on the compiler's vectorizer.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut lanes = [0f32; 8];
    let (a8, a_rest) = a.split_at(a.len() - a.len() % 8);
    let (b8, b_rest) = b.split_at(a8.len());
    for (x, y) in a8.chunks_exact(8).zip(b8.chunks_exact(8)) {
        for l in 0..8 {
            lanes[l] += x[l] * y[l];
        }
    }
    let mut sum = ((lanes[0] + lanes[4]) + (lanes[1] + lanes[5]))
        + ((lanes[2] + lanes[6]) + (lanes[3] + lanes[7]));
    for (x, y) in a_rest.iter().zip(b_rest) {
        sum += x * y;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lanes_add_up_to_the_plain_dot_product() {
        for dims in [1, 7, 8, 9, 512, 513] {
            let a: Vec<f32> = (0..dims).map(|i| (i as f32 * 0.37).sin()).collect();
            let b: Vec<f32> = (0..dims).map(|i| (i as f32 * 0.11).cos()).collect();
            let plain: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
            assert!((dot(&a, &b) - plain).abs() < 1e-3, "{dims} dims");
        }
    }

    #[test]
    fn centring_takes_out_what_every_point_shares() {
        // Three points that all lean along the first axis.
        let mut rows = vec![
            vec![0.9, 0.3, 0.0],
            vec![0.9, -0.3, 0.1],
            vec![0.9, 0.0, -0.3],
        ];
        for r in &mut rows {
            normalize(r);
        }
        let points = Points::from_rows(3, &rows);
        let before = dot(points.row(0), points.row(1));
        let centred = points.centred();
        let after = dot(centred.row(0), centred.row(1));
        assert!(before > 0.7 && after < 0.0, "{before} then {after}");
        for i in 0..3 {
            let len = dot(centred.row(i), centred.row(i));
            assert!((len - 1.0).abs() < 1e-5);
        }
    }
}
