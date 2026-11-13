//! Made-up vectors with a known answer, for the tests and for the
//! measurement of what the spike costs. Never part of a result.

use super::Points;
use super::rng::Rng;
use crate::services::embedding::vector::normalize;

/// `groups` × `each` unit vectors around random centres, group after
/// group; `noise` is how far a member strays from its centre.
pub fn planted(groups: usize, each: usize, dims: usize, noise: f32, seed: u64) -> Points {
    let mut rng = Rng::new(seed);
    let mut points = Points::new(dims);
    for _ in 0..groups {
        let centre: Vec<f32> = (0..dims).map(|_| rng.normal()).collect();
        for _ in 0..each {
            let mut row: Vec<f32> = centre.iter().map(|c| c + noise * rng.normal()).collect();
            normalize(&mut row);
            points.push(&row);
        }
    }
    points
}
