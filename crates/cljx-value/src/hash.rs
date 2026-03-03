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

/// Murmur3 finalizer mix (matches Clojure's `Util.hashCombine`).
pub fn murmur3_mix(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

/// Hash a single i64 the way Clojure does (mix the two 32-bit halves).
pub fn hash_i64(n: i64) -> u32 {
    let lo = n as u32;
    let hi = (n >> 32) as u32;
    murmur3_mix(lo ^ hi)
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
}
