//! A minimal soft-wrapping text editor for editing a single cell.

use unicode_width::UnicodeWidthChar;

pub struct Editor {
    chars: Vec<char>,
    cursor: usize,
}

/// Wrapped text ready for display, with the cursor position.
pub struct Layout {
    pub lines: Vec<String>,
    pub cursor: (u16, u16),
}

/// A display line: chars[start..end]. `soft` = ended by wrapping.
struct Span {
    start: usize,
    end: usize,
    soft: bool,
}

fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

impl Editor {
    pub fn new(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        Self {
            cursor: chars.len(),
            chars,
        }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn insert_char(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    pub fn insert_str(&mut self, s: &str) {
        let s = s.replace("\r\n", "\n").replace(['\r', '\t'], " ");
        for c in s.chars().filter(|c| *c == '\n' || !c.is_control()) {
            self.insert_char(c);
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.chars.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.chars.len();
    }

    pub fn kill_to_start(&mut self) {
        self.chars.drain(..self.cursor);
        self.cursor = 0;
    }

    pub fn kill_to_end(&mut self) {
        self.chars.truncate(self.cursor);
    }

    pub fn delete_word_back(&mut self) {
        let mut start = self.cursor;
        while start > 0 && self.chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !self.chars[start - 1].is_whitespace() {
            start -= 1;
        }
        self.chars.drain(start..self.cursor);
        self.cursor = start;
    }

    pub fn up(&mut self, width: usize) {
        self.move_vertical(width, -1);
    }

    pub fn down(&mut self, width: usize) {
        self.move_vertical(width, 1);
    }

    pub fn layout(&self, width: usize) -> Layout {
        let spans = self.spans(width);
        let (x, y) = self.cursor_xy(&spans, width);
        let mut lines: Vec<String> = spans
            .iter()
            .map(|s| self.chars[s.start..s.end].iter().collect())
            .collect();
        if y >= lines.len() {
            lines.push(String::new());
        }
        Layout {
            lines,
            cursor: (x as u16, y as u16),
        }
    }

    fn spans(&self, width: usize) -> Vec<Span> {
        let width = width.max(2);
        let mut spans = Vec::new();
        let (mut start, mut w) = (0, 0);
        for (i, &c) in self.chars.iter().enumerate() {
            if c == '\n' {
                spans.push(Span {
                    start,
                    end: i,
                    soft: false,
                });
                (start, w) = (i + 1, 0);
                continue;
            }
            let cw = char_width(c);
            if w + cw > width && i > start {
                spans.push(Span {
                    start,
                    end: i,
                    soft: true,
                });
                (start, w) = (i, 0);
            }
            w += cw;
        }
        spans.push(Span {
            start,
            end: self.chars.len(),
            soft: false,
        });
        spans
    }

    /// Cursor position in display cells. A cursor at the end of a full
    /// last line wraps to the start of an extra line.
    fn cursor_xy(&self, spans: &[Span], width: usize) -> (usize, usize) {
        let y = spans
            .iter()
            .rposition(|s| s.start <= self.cursor)
            .unwrap_or(0);
        let x: usize = self.chars[spans[y].start..self.cursor]
            .iter()
            .map(|&c| char_width(c))
            .sum();
        if x >= width.max(2) {
            (0, y + 1)
        } else {
            (x, y)
        }
    }

    fn move_vertical(&mut self, width: usize, delta: isize) {
        let spans = self.spans(width);
        let (x, y) = self.cursor_xy(&spans, width);
        let y = y.min(spans.len() - 1);
        let target = y as isize + delta;
        if target < 0 {
            self.cursor = 0;
            return;
        }
        let Some(span) = spans.get(target as usize) else {
            self.cursor = self.chars.len();
            return;
        };
        // On a soft-wrapped line, the end position belongs to the next line.
        let max = if span.soft { span.end - 1 } else { span.end };
        let (mut pos, mut w) = (span.start, 0);
        while pos < max {
            let cw = char_width(self.chars[pos]);
            if w + cw > x {
                break;
            }
            w += cw;
            pos += 1;
        }
        self.cursor = pos;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_wide_chars() {
        let e = Editor::new("あいうえお");
        let l = e.layout(4);
        assert_eq!(l.lines, ["あい", "うえ", "お"]);
        assert_eq!(l.cursor, (2, 2));
    }

    #[test]
    fn cursor_at_end_of_full_line_moves_to_next_line() {
        let e = Editor::new("abcd");
        let l = e.layout(4);
        assert_eq!(l.lines, ["abcd", ""]);
        assert_eq!(l.cursor, (0, 1));
    }

    #[test]
    fn hard_newlines() {
        let mut e = Editor::new("ab\ncd");
        assert_eq!(e.layout(10).lines, ["ab", "cd"]);
        e.home();
        e.down(10);
        assert_eq!(e.layout(10).cursor, (0, 1));
        e.end();
        e.up(10);
        assert_eq!(e.layout(10).cursor, (2, 0));
    }

    #[test]
    fn vertical_movement_on_soft_wrap() {
        let mut e = Editor::new("abcdefgh");
        e.home();
        e.right();
        e.right();
        e.right(); // after "abc"
        e.down(4);
        assert_eq!(e.layout(4).cursor, (3, 1));
        e.up(4);
        assert_eq!(e.layout(4).cursor, (3, 0));
    }

    #[test]
    fn editing() {
        let mut e = Editor::new("hello world");
        e.delete_word_back();
        assert_eq!(e.text(), "hello ");
        e.insert_str("日本\r\n語\t!");
        assert_eq!(e.text(), "hello 日本\n語 !");
        e.home();
        e.delete();
        e.right();
        e.kill_to_end();
        assert_eq!(e.text(), "e");
        e.kill_to_start();
        assert_eq!(e.text(), "");
        e.backspace();
        assert_eq!(e.text(), "");
    }
}
