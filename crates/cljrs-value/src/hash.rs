/// Clojure-compatible hashing.
///
/// Clojure uses a specific hash algorithm (based on Murmur3) so that:
/// - `(hash 1)` == `(hash 1N)` (Long and BigInt with the same value)
/// - `(hash 1.0)` == `(hash 1)` when the double is a whole number
/// - String hashing matches the JVM `String.hashCode()` algorithm
///
/// We implement a simplified but compatible version here.  Phase 5 can
/// refine this to byte-exact JVM compatibility if needed.
pub trait ClojureHash {
    fn clojure_hash(&self) -> u32;
}

/// Murmur3 finalizer mix (`fmix32`).
pub fn murmur3_mix(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

const MURMUR3_C1: u32 = 0xcc9e_2d51;
const MURMUR3_C2: u32 = 0x1b87_3593;

/// Murmur3 per-block key scramble (`Murmur3.mixK1`).
fn murmur3_mix_k1(mut k1: u32) -> u32 {
    k1 = k1.wrapping_mul(MURMUR3_C1);
    k1 = k1.rotate_left(15);
    k1.wrapping_mul(MURMUR3_C2)
}

/// Murmur3 per-block state update (`Murmur3.mixH1`).
fn murmur3_mix_h1(mut h1: u32, k1: u32) -> u32 {
    h1 ^= k1;
    h1 = h1.rotate_left(13);
    h1.wrapping_mul(5).wrapping_add(0xe654_6b64)
}

/// Murmur3 (x86_32, seed 0) over a sequence of 32-bit words.
///
/// Each word is scrambled and folded into the state separately, so no
/// information is discarded before mixing (unlike XOR-folding the words).
fn murmur3_words(words: &[u32]) -> u32 {
    let mut h1 = 0u32;
    for &w in words {
        h1 = murmur3_mix_h1(h1, murmur3_mix_k1(w));
    }
    murmur3_mix(h1 ^ (words.len() as u32 * 4))
}

/// Hash a single i64 exactly as Clojure's `Murmur3.hashLong` does: Murmur3
/// over the low then high 32-bit word, with `0` hashing to `0`.
pub fn hash_i64(n: i64) -> u32 {
    if n == 0 {
        return 0;
    }
    murmur3_words(&[n as u32, ((n as u64) >> 32) as u32])
}

/// Hash a u128 with Murmur3 over its four 32-bit words (low word first).
pub fn hash_u128(n: u128) -> u32 {
    murmur3_words(&[
        n as u32,
        (n >> 32) as u32,
        (n >> 64) as u32,
        (n >> 96) as u32,
    ])
}

/// Hash a string using the JVM `String.hashCode` algorithm (UTF-16 codepoints).
/// This matches `(hash "foo")` in Clojure.
pub fn hash_string(s: &str) -> u32 {
    let mut h: i32 = 0;
    for ch in s.chars() {
        // Encode as UTF-16 (each char is one or two u16 code units).
        let mut buf = [0u16; 2];
        for unit in ch.encode_utf16(&mut buf) {
            h = h.wrapping_mul(31).wrapping_add(*unit as i32);
        }
    }
    murmur3_mix(h as u32)
}

/// Combine two hashes (order-independent — used for map/set).
pub fn hash_combine_unordered(a: u32, b: u32) -> u32 {
    a ^ b
}

/// Combine two hashes preserving order (used for list/vector).
pub fn hash_combine_ordered(acc: u32, h: u32) -> u32 {
    murmur3_mix(acc.wrapping_mul(31).wrapping_add(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_i64_zero() {
        // Just ensure it doesn't panic and produces a deterministic value.
        let h1 = hash_i64(0);
        let h2 = hash_i64(0);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_string_deterministic() {
        let h1 = hash_string("hello");
        let h2 = hash_string("hello");
        assert_eq!(h1, h2);
        // Different strings have different hashes (with very high probability).
        assert_ne!(hash_string("hello"), hash_string("world"));
    }

    #[test]
    fn test_murmur3_avalanche() {
        // Changing one bit should avalanche.
        let h1 = murmur3_mix(0);
        let h2 = murmur3_mix(1);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_hash_i64_matches_clojure() {
        // Reference values from JVM Clojure's `(hash n)`.
        assert_eq!(hash_i64(0), 0);
        assert_eq!(hash_i64(1), 1392991556);
        assert_eq!(hash_i64(2) as i32, -971005196);
        assert_eq!(hash_i64(-1), 1651860712);
    }

    /// Structured integer families must not collapse onto a few hashes
    /// (issue #507: XOR-folding the words sent every `k * (2^32 + 1)` to 0).
    fn assert_well_distributed(name: &str, inputs: impl Iterator<Item = i64>) {
        let inputs: Vec<i64> = inputs.collect();
        let distinct_inputs: std::collections::HashSet<i64> = inputs.iter().copied().collect();
        let hashes: std::collections::HashSet<u32> =
            distinct_inputs.iter().map(|&n| hash_i64(n)).collect();
        // Allow a tiny number of genuine 32-bit collisions.
        assert!(
            hashes.len() * 1000 >= distinct_inputs.len() * 999,
            "{name}: {} distinct inputs produced only {} distinct hashes",
            distinct_inputs.len(),
            hashes.len()
        );
    }

    #[test]
    fn test_hash_i64_repeated_words() {
        assert_well_distributed(
            "repeated words",
            (0..20_000i64).map(|k| k.wrapping_mul(0x1_0000_0001)),
        );
    }

    #[test]
    fn test_hash_i64_swapped_words() {
        for k in 1..2_000i64 {
            let a = (k << 32) | (k + 1);
            let b = ((k + 1) << 32) | k;
            assert_ne!(hash_i64(a), hash_i64(b), "swapped words collide for k={k}");
        }
        assert_well_distributed(
            "swapped words",
            (1..10_000i64).flat_map(|k| [(k << 32) | (k + 1), ((k + 1) << 32) | k]),
        );
    }

    #[test]
    fn test_hash_i64_powers_of_two() {
        assert_well_distributed("powers of two", (0..64).map(|s| 1i64.wrapping_shl(s)));
        assert_well_distributed("negated powers of two", (0..64).map(|s| -1i64 << s));
    }

    #[test]
    fn test_hash_i64_fixed_strides() {
        for stride in [
            1i64,
            31,
            1 << 16,
            1 << 32,
            (1 << 32) - 1,
            (1 << 32) + 1,
            1 << 48,
        ] {
            assert_well_distributed(
                &format!("stride {stride}"),
                (0..20_000i64).map(|i| i.wrapping_mul(stride)),
            );
        }
    }

    #[test]
    fn test_hash_u128_repeated_and_swapped_words() {
        let mut hashes = std::collections::HashSet::new();
        let mut n = 0usize;
        for k in 1..5_000u128 {
            // All four words equal; pairs of equal words; word order permuted.
            for v in [
                k | (k << 32) | (k << 64) | (k << 96),
                k | (k << 32),
                k | (k << 64),
                k << 96,
                k << 64,
                k << 32,
            ] {
                hashes.insert(hash_u128(v));
                n += 1;
            }
        }
        assert!(hashes.len() * 1000 >= n * 999, "{} of {n}", hashes.len());
    }
}
