//! A small seeded generator (SplitMix64), so areas and layouts are the
//! same for the same seed and no dependency is needed.

#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `n` (which is at least 1).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// A number in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A number in [-1, 1).
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    /// A normally distributed number (for synthetic data in tests and the
    /// measurement; never for a result).
    pub fn normal(&mut self) -> f32 {
        let u = f64::from(self.unit()).max(1e-9);
        let v = f64::from(self.unit());
        ((-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_numbers() {
        let (mut a, mut b, mut c) = (Rng::new(1), Rng::new(1), Rng::new(2));
        let first: Vec<u64> = (0..5).map(|_| a.next_u64()).collect();
        assert_eq!(first, (0..5).map(|_| b.next_u64()).collect::<Vec<_>>());
        assert_ne!(first, (0..5).map(|_| c.next_u64()).collect::<Vec<_>>());
        for _ in 0..1000 {
            let u = a.unit();
            assert!((0.0..1.0).contains(&u));
            assert!(a.below(7) < 7);
        }
    }
}
