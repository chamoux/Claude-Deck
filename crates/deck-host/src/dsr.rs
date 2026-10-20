//! DsrFilter: removes ESC[6n from output and counts the queries, even when split across reads.

/// The cursor-position query, `ESC [ 6 n`.
const QUERY: &[u8] = b"\x1b[6n";

/// Removes cursor-position queries (`ESC [ 6 n`) from PTY output and counts them,
/// so the host can answer them itself (ConPTY waits for the answer) and the
/// window's terminal does not answer a second time.
#[derive(Debug, Default)]
pub struct DsrFilter {
    carry: Vec<u8>,
}

impl DsrFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the bytes to pass on and how many queries were removed. A partial
    /// `ESC`, `ESC [` or `ESC [ 6` at the end is held back until the next call.
    pub fn feed(&mut self, bytes: &[u8]) -> (Vec<u8>, usize) {
        let (out, at) = self.feed_at(bytes);
        (out, at.len())
    }

    /// Like `feed`, but says where each query sat: an offset into the returned
    /// bytes. The cursor to report is the one after processing `out[..offset]`.
    pub fn feed_at(&mut self, bytes: &[u8]) -> (Vec<u8>, Vec<usize>) {
        let mut data = std::mem::take(&mut self.carry);
        data.extend_from_slice(bytes);

        let mut out = Vec::with_capacity(data.len());
        let mut at = Vec::new();
        let mut i = 0;
        while i < data.len() {
            let rest = &data[i..];
            if rest.starts_with(QUERY) {
                at.push(out.len());
                i += QUERY.len();
            } else if rest.len() < QUERY.len() && QUERY.starts_with(rest) {
                // A possible query cut off by the end of this read: wait for more bytes.
                self.carry = rest.to_vec();
                break;
            } else {
                out.push(data[i]);
                i += 1;
            }
        }
        (out, at)
    }
}

/// The answer to one query: `ESC [ row ; col R`, 1-based.
pub fn cursor_report(row0: u16, col0: u16) -> Vec<u8> {
    format!("\x1b[{};{}R", u32::from(row0) + 1, u32::from(col0) + 1).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passes_through_unchanged() {
        let mut f = DsrFilter::new();
        assert_eq!(f.feed(b"hello world"), (b"hello world".to_vec(), 0));
    }

    #[test]
    fn one_query_in_the_middle_is_removed() {
        let mut f = DsrFilter::new();
        assert_eq!(f.feed(b"ab\x1b[6ncd"), (b"abcd".to_vec(), 1));
    }

    #[test]
    fn two_queries_in_one_chunk_are_both_counted() {
        let mut f = DsrFilter::new();
        let (out, n) = f.feed(b"\x1b[6nx\x1b[6n");
        assert_eq!(out, b"x".to_vec());
        assert_eq!(n, 2);
    }

    #[test]
    fn query_split_after_escape_is_held_then_removed() {
        let mut f = DsrFilter::new();
        assert_eq!(f.feed(b"x\x1b"), (b"x".to_vec(), 0));
        assert_eq!(f.feed(b"[6ny"), (b"y".to_vec(), 1));
    }

    #[test]
    fn query_split_after_escape_bracket_is_held_then_removed() {
        let mut f = DsrFilter::new();
        assert_eq!(f.feed(b"\x1b["), (Vec::new(), 0));
        assert_eq!(f.feed(b"6n"), (Vec::new(), 1));
    }

    #[test]
    fn query_split_after_escape_bracket_six_is_held_then_removed() {
        let mut f = DsrFilter::new();
        assert_eq!(f.feed(b"\x1b[6"), (Vec::new(), 0));
        assert_eq!(f.feed(b"n"), (Vec::new(), 1));
    }

    #[test]
    fn non_query_sgr_sequence_passes_through_unchanged() {
        let mut f = DsrFilter::new();
        assert_eq!(f.feed(b"\x1b[6m"), (b"\x1b[6m".to_vec(), 0));
    }

    #[test]
    fn non_query_sgr_sequence_split_after_six_passes_through_unchanged() {
        let mut f = DsrFilter::new();
        assert_eq!(f.feed(b"\x1b[6"), (Vec::new(), 0));
        assert_eq!(f.feed(b"m"), (b"\x1b[6m".to_vec(), 0));
    }

    #[test]
    fn trailing_lone_escape_is_released_when_next_byte_is_not_a_query() {
        let mut f = DsrFilter::new();
        assert_eq!(f.feed(b"\x1b"), (Vec::new(), 0));
        assert_eq!(f.feed(b"A"), (b"\x1bA".to_vec(), 0));
    }

    #[test]
    fn feed_at_gives_each_query_offset_in_the_clean_bytes() {
        let mut f = DsrFilter::new();
        assert_eq!(
            f.feed_at(b"\x1b[6nab\x1b[6ncd\x1b[6n"),
            (b"abcd".to_vec(), vec![0, 2, 4])
        );
        // A query split across reads sits at the start of the second read.
        assert_eq!(f.feed_at(b"xy\x1b["), (b"xy".to_vec(), vec![]));
        assert_eq!(f.feed_at(b"6nz"), (b"z".to_vec(), vec![0]));
    }

    #[test]
    fn cursor_report_is_one_based() {
        assert_eq!(cursor_report(0, 0), b"\x1b[1;1R".to_vec());
        assert_eq!(cursor_report(4, 9), b"\x1b[5;10R".to_vec());
    }
}
