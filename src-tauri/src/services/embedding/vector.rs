//! Vector arithmetic and the catalog encoding: unit-length vectors stored as
//! little-endian f16 BLOBs (ADR 0013).

use half::f16;

/// Scales `v` to unit length. `false` (and `v` unchanged) when it has no
/// length or holds a non-finite number.
pub fn normalize(v: &mut [f32]) -> bool {
    if v.iter().any(|x| !x.is_finite()) {
        return false;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 || !norm.is_finite() {
        return false;
    }
    for x in v.iter_mut() {
        *x /= norm;
    }
    true
}

/// The first `dims` numbers of `v`, at unit length (Matryoshka truncation).
pub fn truncate_normalize(v: &[f32], dims: usize) -> Option<Vec<f32>> {
    let mut out = v.get(..dims)?.to_vec();
    normalize(&mut out).then_some(out)
}

pub fn to_f16_blob(v: &[f32]) -> Vec<u8> {
    v.iter()
        .flat_map(|x| f16::from_f32(*x).to_le_bytes())
        .collect()
}

/// `None` unless `blob` holds exactly `dims` numbers.
pub fn from_f16_blob(blob: &[u8], dims: usize) -> Option<Vec<f32>> {
    (blob.len() == dims * 2).then(|| {
        blob.chunks_exact(2)
            .map(|b| f16::from_le_bytes([b[0], b[1]]).to_f32())
            .collect()
    })
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_rejects_what_has_no_direction() {
        let mut v = vec![3.0, 4.0];
        assert!(normalize(&mut v));
        assert_eq!(v, [0.6, 0.8]);
        assert!(!normalize(&mut [0.0, 0.0]));
        assert!(!normalize(&mut [f32::NAN, 1.0]));
        assert!(!normalize(&mut [f32::INFINITY, 1.0]));
        let t = truncate_normalize(&[3.0, 4.0, 12.0], 2).unwrap();
        assert_eq!(t, [0.6, 0.8]);
        assert!(truncate_normalize(&[1.0], 2).is_none());
    }

    #[test]
    fn f16_blobs_round_trip_within_half_precision() {
        let mut v: Vec<f32> = (0..1024).map(|i| ((i * 37) % 101) as f32 - 50.0).collect();
        assert!(normalize(&mut v));
        let blob = to_f16_blob(&v);
        assert_eq!(blob.len(), 2048);
        let back = from_f16_blob(&blob, 1024).unwrap();
        for (a, b) in v.iter().zip(&back) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
        assert!((dot(&back, &back) - 1.0).abs() < 1e-3);
        assert!(from_f16_blob(&blob, 1023).is_none());
        assert!(from_f16_blob(&blob[..2047], 1024).is_none());
    }
}
