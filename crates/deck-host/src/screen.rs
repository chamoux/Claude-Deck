//! Screen: headless terminal (vt100) with resize and snapshot.

use std::num::NonZeroU16;

/// Headless terminal for one PTY, so a window can reattach and redraw (spec §3.2).
pub struct Screen {
    parser: vt100::Parser,
}

fn nz(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap_or(NonZeroU16::MIN)
}

impl Screen {
    /// Sizes below 1 are treated as 1. `scrollback` is the number of history lines kept.
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        Self {
            parser: vt100::Parser::new(nz(rows), nz(cols), scrollback),
        }
    }
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.set_size(nz(rows), nz(cols));
    }
    pub fn size(&self) -> (u16, u16) {
        let (r, c) = self.parser.screen().size();
        (r.get(), c.get())
    }
    /// Zero-based (row, col) of the cursor.
    pub fn cursor(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }
    pub fn alternate_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }
    /// Bytes that redraw scrollback, then the screen, then the cursor and modes, in a fresh terminal of the same size.
    pub fn snapshot(&mut self) -> Vec<u8> {
        if self.parser.screen().alternate_screen() {
            // The main screen's scrollback is unreachable while the alternate screen is active.
            let mut out = b"\x1b[2J\x1b[H\x1b[?1049h".to_vec();
            out.extend_from_slice(self.parser.screen().state_formatted().as_bytes());
            return out;
        }
        let (rows, cols) = self.size();
        let sb = self.parser.screen().scrollback_len();
        let mut out = Vec::new();
        out.extend_from_slice(b"\x1b[?25l\x1b[m\x1b[2J\x1b[H");
        for i in 0..sb {
            // Oldest first: with offset sb - i, the top visible row is scrollback row i.
            self.parser.screen_mut().set_scrollback(sb - i);
            if let Some(row) = self.parser.screen().rows_formatted(0, cols).next() {
                out.extend_from_slice(row.as_bytes());
            }
            out.extend_from_slice(b"\x1b[m\r\n");
        }
        if sb > 0 {
            out.extend(std::iter::repeat_n(b'\n', usize::from(rows) - 1));
        }
        self.parser.screen_mut().set_scrollback(0);
        out.extend_from_slice(b"\x1b[m\x1b[2J\x1b[H");
        out.extend_from_slice(self.parser.screen().state_formatted().as_bytes());
        out
    }
    /// Plain text of the visible screen.
    pub fn contents(&self) -> String {
        self.parser.screen().contents()
    }

    #[cfg(test)]
    fn scrollback_len(&self) -> usize {
        self.parser.screen().scrollback_len()
    }

    /// Plain text of every scrollback row, oldest first.
    #[cfg(test)]
    fn scrollback_rows(&mut self) -> Vec<String> {
        let sb = self.scrollback_len();
        let mut rows = Vec::new();
        for i in 0..sb {
            self.parser.screen_mut().set_scrollback(sb - i);
            rows.push(
                self.parser
                    .screen()
                    .contents()
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string(),
            );
        }
        self.parser.screen_mut().set_scrollback(0);
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_reproduces_screen_scrollback_and_cursor() {
        let mut a = Screen::new(10, 40, 5000);
        for n in 0..25 {
            a.process(format!("\x1b[3{}mline {n}\x1b[m\r\n", n % 7 + 1).as_bytes());
        }
        a.process(b"\x1b[1mbold\x1b[m \x1b[4munder\x1b[m \x1b[38;2;10;200;30mrgb\x1b[m");
        a.process(b"\x1b[3;5HXY");
        let snap = a.snapshot();
        let mut b = Screen::new(10, 40, 5000);
        b.process(&snap);
        assert_eq!(a.contents(), b.contents());
        assert_eq!(a.cursor(), b.cursor());
        assert_eq!(a.scrollback_len(), b.scrollback_len());
        assert!(a.scrollback_len() > 0);
        assert_eq!(a.scrollback_rows(), b.scrollback_rows());
    }

    #[test]
    fn snapshot_restores_the_live_view() {
        let mut a = Screen::new(5, 20, 5000);
        for n in 0..12 {
            a.process(format!("row {n}\r\n").as_bytes());
        }
        let before = a.contents();
        let _ = a.snapshot();
        assert_eq!(before, a.contents());
    }

    #[test]
    fn resize_changes_size() {
        let mut a = Screen::new(10, 40, 5000);
        a.resize(20, 80);
        assert_eq!(a.size(), (20, 80));
    }

    #[test]
    fn zero_sizes_become_one() {
        let mut a = Screen::new(0, 0, 5000);
        assert_eq!(a.size(), (1, 1));
        a.resize(0, 5);
        assert_eq!(a.size(), (1, 5));
    }

    #[test]
    fn alternate_screen_snapshot_enters_the_alternate_screen() {
        let mut a = Screen::new(10, 40, 5000);
        a.process(b"\x1b[?1049hhello");
        let snap = a.snapshot();
        let mut b = Screen::new(10, 40, 5000);
        b.process(&snap);
        assert!(b.alternate_screen());
        assert_eq!(a.contents(), b.contents());
    }
}
