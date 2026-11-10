//! A deterministic embedding backend for tests, benchmarks and the smoke
//! test (phase2-spec §3, §11): each lowercase word other than the common
//! ones is hashed into a signed bucket, and the sum is scaled to unit
//! length. Texts that share words are near each other; no model or
//! network is involved.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::vector::normalize;
use crate::domain::traits::{BackendInfo, EmbedError, EmbeddingBackend};

pub const FAKE_MODEL: &str = "atlas-hashed-bag-of-words";

#[derive(Debug)]
pub struct HashedBagOfWordsBackend {
    dims: usize,
    calls: AtomicUsize,
    inputs: AtomicUsize,
}

impl HashedBagOfWordsBackend {
    pub fn new(dims: usize) -> Self {
        assert!(dims > 0, "a vector has at least one number");
        Self {
            dims,
            calls: AtomicUsize::new(0),
            inputs: AtomicUsize::new(0),
        }
    }

    /// How many `embed` calls, and how many texts in them, so far.
    pub fn counts(&self) -> (usize, usize) {
        (
            self.calls.load(Ordering::Relaxed),
            self.inputs.load(Ordering::Relaxed),
        )
    }

    pub fn vector(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dims];
        // Common words carry no meaning (as for a real model, and as the
        // any-words search form drops them).
        for word in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty() && !crate::services::search::query::is_common_word(w))
        {
            let h = blake3::hash(word.to_lowercase().as_bytes());
            let b = h.as_bytes();
            let bucket = u64::from_le_bytes(b[..8].try_into().expect("8 bytes"));
            let sign = if b[8] & 1 == 0 { 1.0 } else { -1.0 };
            v[(bucket % self.dims as u64) as usize] += sign;
        }
        if !normalize(&mut v) {
            // No words (or they cancelled out): a fixed direction.
            v.fill(0.0);
            v[0] = 1.0;
        }
        v
    }
}

impl EmbeddingBackend for HashedBagOfWordsBackend {
    fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.inputs.fetch_add(inputs.len(), Ordering::Relaxed);
        Ok(inputs.iter().map(|t| self.vector(t)).collect())
    }

    fn describe(&self) -> BackendInfo {
        BackendInfo {
            model: FAKE_MODEL.to_owned(),
            dims: self.dims,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::embedding::vector::dot;

    #[test]
    fn deterministic_unit_vectors_that_follow_shared_words() {
        let b = HashedBagOfWordsBackend::new(256);
        let v = b
            .embed(&[
                "The orbit of Mars",
                "the ORBIT of mars!",
                "Baking sourdough bread",
                "",
            ])
            .unwrap();
        assert_eq!(v.len(), 4);
        assert_eq!(v[0], v[1], "case and punctuation don't matter");
        for x in &v {
            assert_eq!(x.len(), 256);
            assert!((dot(x, x) - 1.0).abs() < 1e-5);
        }
        let near = dot(&v[0], &b.vector("Mars has an elliptical orbit"));
        let far = dot(&v[0], &v[2]);
        assert!(near > far + 0.2, "near {near}, far {far}");
        assert_eq!(v[3][0], 1.0, "empty text has a fixed direction");
        assert_eq!(b.counts(), (1, 4));
        assert_eq!(b.describe().dims, 256);
    }
}
