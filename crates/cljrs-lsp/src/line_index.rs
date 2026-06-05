//! Byte-offset ⇄ LSP [`Position`] conversion.
//!
//! The reader reports byte offsets (and a *byte* column that is useless for
//! LSP). LSP positions are 0-based `(line, character)` pairs where `character`
//! counts code units in the negotiated [`OffsetEncoding`] — UTF-16 by default,
//! UTF-8 when the client advertises support. We derive everything from byte
//! offsets via a precomputed table of line-start offsets.

use tower_lsp::lsp_types::{Position, Range};

/// The unit in which LSP `character` offsets are counted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OffsetEncoding {
    /// `character` counts UTF-8 bytes (LSP 3.17 `utf-8`).
    Utf8,
    /// `character` counts UTF-16 code units (the LSP default).
    Utf16,
}

/// Precomputed line boundaries for one document, borrowing its text.
pub struct LineIndex<'a> {
    text: &'a str,
    /// Byte offset of the first character of each line. Always starts with `0`.
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self { text, line_starts }
    }

    /// Convert an absolute byte offset into an LSP [`Position`].
    ///
    /// Offsets past the end of the text are clamped to the text length, and
    /// offsets that do not land on a `char` boundary are snapped down to the
    /// nearest boundary (defensive — reader spans are always on boundaries).
    pub fn position(&self, offset: usize, encoding: OffsetEncoding) -> Position {
        let mut offset = offset.min(self.text.len());
        while offset > 0 && !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(insert) => insert - 1,
        };
        let line_start = self.line_starts[line];
        let character = match encoding {
            OffsetEncoding::Utf8 => (offset - line_start) as u32,
            OffsetEncoding::Utf16 => self.text[line_start..offset]
                .chars()
                .map(|c| c.len_utf16() as u32)
                .sum(),
        };
        Position {
            line: line as u32,
            character,
        }
    }

    /// Convert a half-open byte range `[start, end)` into an LSP [`Range`].
    pub fn range(&self, start: usize, end: usize, encoding: OffsetEncoding) -> Range {
        Range {
            start: self.position(start, encoding),
            end: self.position(end, encoding),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_positions() {
        let text = "abc\ndef\n";
        let li = LineIndex::new(text);
        assert_eq!(li.position(0, OffsetEncoding::Utf8), Position::new(0, 0));
        assert_eq!(li.position(2, OffsetEncoding::Utf8), Position::new(0, 2));
        // offset 3 is the '\n' itself (end of line 0)
        assert_eq!(li.position(3, OffsetEncoding::Utf8), Position::new(0, 3));
        // offset 4 is start of line 1
        assert_eq!(li.position(4, OffsetEncoding::Utf8), Position::new(1, 0));
        assert_eq!(li.position(6, OffsetEncoding::Utf8), Position::new(1, 2));
    }

    #[test]
    fn non_ascii_utf8_vs_utf16() {
        // "λ" is 2 UTF-8 bytes, 1 UTF-16 unit. "😀" is 4 bytes, 2 UTF-16 units.
        let text = "λ😀x";
        let li = LineIndex::new(text);
        let x_off = "λ😀".len(); // byte offset of 'x' == 6
        assert_eq!(x_off, 6);
        // UTF-8: character == byte offset within the line.
        assert_eq!(
            li.position(x_off, OffsetEncoding::Utf8),
            Position::new(0, 6)
        );
        // UTF-16: λ(1) + 😀(2) == 3 code units before 'x'.
        assert_eq!(
            li.position(x_off, OffsetEncoding::Utf16),
            Position::new(0, 3)
        );
    }

    #[test]
    fn clamps_past_eof() {
        let text = "ab";
        let li = LineIndex::new(text);
        assert_eq!(li.position(999, OffsetEncoding::Utf8), Position::new(0, 2));
    }

    #[test]
    fn snaps_off_boundary_down() {
        let text = "λ"; // 2 bytes
        let li = LineIndex::new(text);
        // offset 1 is inside the multibyte char → snaps to 0.
        assert_eq!(li.position(1, OffsetEncoding::Utf16), Position::new(0, 0));
    }

    #[test]
    fn crlf_keeps_cr_in_line() {
        let text = "a\r\nb";
        let li = LineIndex::new(text);
        // '\r' is at offset 1, still on line 0 at column 1.
        assert_eq!(li.position(1, OffsetEncoding::Utf8), Position::new(0, 1));
        // 'b' is at offset 3, start of line 1.
        assert_eq!(li.position(3, OffsetEncoding::Utf8), Position::new(1, 0));
    }

    #[test]
    fn range_round_trip() {
        let text = "(def x 1)";
        let li = LineIndex::new(text);
        let r = li.range(1, 4, OffsetEncoding::Utf8); // "def"
        assert_eq!(r.start, Position::new(0, 1));
        assert_eq!(r.end, Position::new(0, 4));
    }
}
