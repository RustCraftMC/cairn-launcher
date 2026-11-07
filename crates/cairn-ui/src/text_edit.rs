//! Pure logic for single-line text editing: content, cursor, selection. No drawing or IME.
//! Positions are UTF-8 byte offsets that always sit on grapheme cluster boundaries,
//! so emoji and combining marks are never split in half.

use std::ops::Range;

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EditState {
    text: String,
    cursor: usize,
    /// The other end of the selection. Equal to `cursor` when nothing is selected.
    anchor: usize,
}

fn prev_boundary(s: &str, i: usize) -> usize {
    let mut c = GraphemeCursor::new(i, s.len(), true);
    c.prev_boundary(s, 0).ok().flatten().unwrap_or(0)
}

fn next_boundary(s: &str, i: usize) -> usize {
    let mut c = GraphemeCursor::new(i, s.len(), true);
    c.next_boundary(s, 0).ok().flatten().unwrap_or(s.len())
}

/// Snaps a byte offset down to the nearest grapheme cluster boundary at or before it.
fn snap(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    let mut c = GraphemeCursor::new(i, s.len(), true);
    if c.is_boundary(s, 0).unwrap_or(true) { i } else { prev_boundary(s, i) }
}

fn is_space(seg: &str) -> bool {
    seg.chars().all(char::is_whitespace)
}

fn word_left(s: &str, i: usize) -> usize {
    s.split_word_bound_indices().rev().find(|(start, seg)| *start < i && !is_space(seg)).map(|(start, _)| start).unwrap_or(0)
}

/// Start of the next word, as on Windows. Goes to the end if no word follows.
fn word_right(s: &str, i: usize) -> usize {
    s.split_word_bound_indices().find(|(start, seg)| *start > i && !is_space(seg)).map(|(start, _)| start).unwrap_or(s.len())
}

impl EditState {
    pub fn new(text: String) -> Self {
        let len = text.len();
        EditState { text, cursor: len, anchor: len }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selection(&self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }

    pub fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    /// Replaces all content, e.g. when the app changes the value from outside.
    /// Keeps the cursor in place if possible, else moves it to the end. Clears the selection.
    pub fn replace_text(&mut self, text: String) {
        let cursor = self.cursor.min(text.len());
        self.text = text;
        self.set_cursor(cursor, false);
    }

    pub fn set_cursor(&mut self, index: usize, extend: bool) {
        self.cursor = snap(&self.text, index);
        if !extend {
            self.anchor = self.cursor;
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    pub fn select_word_at(&mut self, index: usize) {
        let index = snap(&self.text, index);
        let seg =
            self.text.split_word_bound_indices().find(|(start, seg)| index < start + seg.len()).or_else(|| self.text.split_word_bound_indices().next_back());
        if let Some((start, seg)) = seg {
            self.anchor = start;
            self.cursor = start + seg.len();
        }
    }

    pub fn insert(&mut self, s: &str) {
        let r = self.selection();
        self.text.replace_range(r.clone(), s);
        self.cursor = r.start + s.len();
        self.anchor = self.cursor;
    }

    pub fn delete_selection(&mut self) -> bool {
        if !self.has_selection() {
            return false;
        }
        self.insert("");
        true
    }

    /// Backspace. With `word` set, deletes back to the start of the previous word.
    /// Returns whether the content changed.
    pub fn backspace(&mut self, word: bool) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor == 0 {
            return false;
        }
        let start = if word { word_left(&self.text, self.cursor) } else { prev_boundary(&self.text, self.cursor) };
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.anchor = start;
        true
    }

    /// Forward delete. With `word` set, deletes up to the start of the next word.
    /// Returns whether the content changed.
    pub fn delete_forward(&mut self, word: bool) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor >= self.text.len() {
            return false;
        }
        let end = if word { word_right(&self.text, self.cursor) } else { next_boundary(&self.text, self.cursor) };
        self.text.replace_range(self.cursor..end, "");
        true
    }

    pub fn move_left(&mut self, word: bool, extend: bool) {
        if self.has_selection() && !extend && !word {
            let to = self.selection().start;
            return self.set_cursor(to, false);
        }
        let to = if word { word_left(&self.text, self.cursor) } else { prev_boundary(&self.text, self.cursor) };
        self.set_cursor(to, extend);
    }

    pub fn move_right(&mut self, word: bool, extend: bool) {
        if self.has_selection() && !extend && !word {
            let to = self.selection().end;
            return self.set_cursor(to, false);
        }
        let to = if word { word_right(&self.text, self.cursor) } else { next_boundary(&self.text, self.cursor) };
        self.set_cursor(to, extend);
    }

    pub fn home(&mut self, extend: bool) {
        self.set_cursor(0, extend);
    }

    pub fn end(&mut self, extend: bool) {
        self.set_cursor(self.text.len(), extend);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace_by_grapheme() {
        let mut e = EditState::new(String::new());
        e.insert("你好");
        e.insert("👍🏽"); // Skin-tone emoji is one grapheme cluster made of two code points.
        e.insert("e\u{301}"); // `e` plus a combining accent (U+0301).
        assert_eq!(e.text(), "你好👍🏽e\u{301}");
        assert!(e.backspace(false));
        assert_eq!(e.text(), "你好👍🏽");
        assert!(e.backspace(false));
        assert_eq!(e.text(), "你好");
        e.move_left(false, false);
        assert_eq!(e.cursor(), "你".len());
        assert!(e.delete_forward(false));
        assert_eq!(e.text(), "你");
        assert!(!e.delete_forward(false));
    }

    #[test]
    fn selection_replace_and_collapse() {
        let mut e = EditState::new("hello world".into());
        e.home(false);
        e.move_right(true, true); // Ctrl+Shift+Right selects to the start of the next word.
        assert_eq!(e.selected_text(), "hello ");
        e.insert("bye ");
        assert_eq!(e.text(), "bye world");
        e.select_all();
        e.move_left(false, false);
        assert_eq!(e.cursor(), 0);
        assert!(!e.has_selection());
        e.end(false);
        e.move_left(false, true);
        e.move_left(false, true);
        assert_eq!(e.selected_text(), "ld");
        e.move_right(false, false);
        assert_eq!((e.cursor(), e.has_selection()), (e.text().len(), false));
    }

    #[test]
    fn word_motion_and_deletion() {
        let mut e = EditState::new("foo bar, baz".into());
        e.move_left(true, false);
        assert_eq!(e.cursor(), 9);
        e.move_left(true, false);
        assert_eq!(e.cursor(), 7);
        e.move_left(true, false);
        assert_eq!(e.cursor(), 4);
        e.move_right(true, false);
        assert_eq!(e.cursor(), 7);
        e.end(false);
        assert!(e.backspace(true));
        assert_eq!(e.text(), "foo bar, ");
        e.home(false);
        assert!(e.delete_forward(true));
        assert_eq!(e.text(), "bar, ");
    }

    #[test]
    fn select_word_and_snap() {
        let mut e = EditState::new("中文 input".into());
        e.select_word_at(8);
        assert_eq!(e.selected_text(), "input");
        e.select_word_at(1); // Lands inside a multi-byte character, so it snaps to a boundary first.
        assert_eq!(e.selected_text(), "中");
        e.set_cursor(100, false);
        assert_eq!(e.cursor(), e.text().len());
        e.replace_text("中".into());
        assert_eq!((e.cursor(), e.has_selection()), (3, false));
        e.replace_text(String::new());
        assert_eq!(e.cursor(), 0);
    }
}
