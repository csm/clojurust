/// Bits consumed per trie level.
pub const BITS: u32 = 5;
/// Branching factor (2^5 = 32).
pub const WIDTH: usize = 1 << BITS;
/// Mask for extracting one level's worth of bits.
pub const MASK: u32 = (WIDTH as u32) - 1;

/// Extract the index fragment at `depth` levels from the root.
///
/// `depth = 0` is the root level (uses the most-significant fragment),
/// but in practice we shift by `(max_depth - depth) * BITS`.
/// Callers pass a pre-computed `shift = (max_depth - depth) * BITS`.
#[inline]
pub fn fragment(hash: u32, shift: u32) -> u32 {
    (hash >> shift) & MASK
}

/// Given a sparse bitmap and a bit position, compute the dense-array index
/// using a popcount of all set bits below `bit`.
#[inline]
pub fn sparse_index(bitmap: u32, bit: u32) -> usize {
    (bitmap & (bit - 1)).count_ones() as usize
}

/// The single-bit mask for a hash fragment.
#[inline]
pub fn bit_for(frag: u32) -> u32 {
    1 << frag
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fragment() {
        // fragment of 0b11111_00000 at shift=0 → 0b00000 = 0
        assert_eq!(fragment(0b11111_00000, 0), 0);
        // fragment of 0b11111_00000 at shift=5 → 0b11111 = 31
        assert_eq!(fragment(0b11111_00000, 5), 31);
    }

    #[test]
    fn test_sparse_index() {
        // bitmap = 0b1010, bit = 0b0010 (position 1)
        // bits below position 1: 0b1010 & 0b0001 = 0 → index 0
        assert_eq!(sparse_index(0b1010, 0b0010), 0);
        // bitmap = 0b1110, bit = 0b1000 (position 3)
        // bits below: 0b1110 & 0b0111 = 0b0110 → popcount = 2
        assert_eq!(sparse_index(0b1110, 0b1000), 2);
    }
}
