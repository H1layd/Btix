use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Hard cap on the size of a file we will load into a buffer. Keeps a single
/// huge (or malicious) file from exhausting memory.
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// A single open file: its text, cursor, and scroll state.
pub struct Buffer {
    pub path: Option<PathBuf>,
    pub name: String,
    pub lines: Vec<String>,
    pub cx: usize,      // cursor position in characters within the line
    pub cy: usize,      // cursor line number
    pub row_off: usize, // vertical scroll
    pub col_off: usize, // horizontal scroll
    pub dirty: bool,
    pub sel_anchor: Option<(usize, usize)>, // selection anchor (row, col); None = no selection
    pub undo: Vec<(Vec<String>, usize, usize)>, // Ctrl+Z history: (lines, cy, cx)
    pub last_was_typing: bool,                  // groups consecutively typed characters
    pub error_line: Option<usize>,             // 0-based line flagged by the syntax checker
    pub error_col: Option<usize>,              // 0-based column flagged by the syntax checker
}

impl Buffer {
    pub fn from_path(path: PathBuf) -> io::Result<Self> {
        // Read bytes and validate before treating the file as text. A file that
        // exists but is unreadable, too large, or not valid UTF-8 must NOT
        // silently become an empty buffer — otherwise a later save would
        // overwrite real data with nothing. Only a genuinely missing file
        // becomes a fresh empty buffer.
        let content = match fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() as u64 > MAX_FILE_BYTES {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
                }
                String::from_utf8(bytes)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not valid UTF-8"))?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let mut lines: Vec<String> = content.split('\n').map(|s| s.to_string()).collect();
        // For a file ending in \n, split('\n') yields an empty tail — that is the last line.
        if lines.is_empty() {
            lines.push(String::new());
        }
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string());
        Ok(Buffer {
            path: Some(path),
            name,
            lines,
            cx: 0,
            cy: 0,
            row_off: 0,
            col_off: 0,
            dirty: false,
            sel_anchor: None,
            undo: Vec::new(),
            last_was_typing: false,
            error_line: None,
            error_col: None,
        })
    }

    pub fn new_named(name: String, path: Option<PathBuf>) -> Self {
        Buffer {
            path,
            name,
            lines: vec![String::new()],
            cx: 0,
            cy: 0,
            row_off: 0,
            col_off: 0,
            dirty: false,
            sel_anchor: None,
            undo: Vec::new(),
            last_was_typing: false,
            error_line: None,
            error_col: None,
        }
    }

    pub fn extension(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.extension())
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn save(&mut self) -> io::Result<()> {
        let path = match &self.path {
            Some(p) => p.clone(),
            None => return Err(io::Error::new(io::ErrorKind::Other, "no file name")),
        };
        let text = self.lines.join("\n");

        // Atomic save: write to a temp file in the same directory, flush it to
        // disk, then rename over the target. A crash or full disk mid-write
        // leaves the original file intact instead of truncated/corrupted.
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let tmp = dir.join(format!(".{}.btix-tmp", self.name));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        // Preserve the original file's permissions when overwriting.
        if let Ok(meta) = fs::metadata(&path) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        if let Err(e) = fs::rename(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        self.dirty = false;
        Ok(())
    }

    fn line_chars(&self, row: usize) -> Vec<char> {
        self.lines[row].chars().collect()
    }

    pub fn cur_len(&self) -> usize {
        self.lines[self.cy].chars().count()
    }

    fn clamp_cx(&mut self) {
        let len = self.cur_len();
        if self.cx > len {
            self.cx = len;
        }
    }

    // ---- editing ----

    pub fn insert_char(&mut self, c: char) {
        let mut chars = self.line_chars(self.cy);
        let at = self.cx.min(chars.len());
        chars.insert(at, c);
        self.lines[self.cy] = chars.into_iter().collect();
        self.cx = at + 1;
        self.dirty = true;
    }

    pub fn insert_newline(&mut self) {
        let chars = self.line_chars(self.cy);
        let at = self.cx.min(chars.len());
        let head: String = chars[..at].iter().collect();
        let tail: String = chars[at..].iter().collect();
        self.lines[self.cy] = head;
        self.lines.insert(self.cy + 1, tail);
        self.cy += 1;
        self.cx = 0;
        self.dirty = true;
    }

    pub fn backspace(&mut self) {
        if self.cx > 0 {
            let mut chars = self.line_chars(self.cy);
            chars.remove(self.cx - 1);
            self.lines[self.cy] = chars.into_iter().collect();
            self.cx -= 1;
            self.dirty = true;
        } else if self.cy > 0 {
            let cur = self.lines.remove(self.cy);
            self.cy -= 1;
            self.cx = self.cur_len();
            self.lines[self.cy].push_str(&cur);
            self.dirty = true;
        }
    }

    pub fn delete_forward(&mut self) {
        let len = self.cur_len();
        if self.cx < len {
            let mut chars = self.line_chars(self.cy);
            chars.remove(self.cx);
            self.lines[self.cy] = chars.into_iter().collect();
            self.dirty = true;
        } else if self.cy + 1 < self.lines.len() {
            let next = self.lines.remove(self.cy + 1);
            self.lines[self.cy].push_str(&next);
            self.dirty = true;
        }
    }

    // ---- undo ----

    /// Saves the current state into the history.
    pub fn push_undo(&mut self) {
        if self.undo.len() >= 500 {
            self.undo.remove(0);
        }
        self.undo.push((self.lines.clone(), self.cy, self.cx));
    }

    /// Reverts the most recent change.
    pub fn undo(&mut self) {
        if let Some((lines, cy, cx)) = self.undo.pop() {
            self.lines = lines;
            self.cy = cy.min(self.lines.len().saturating_sub(1));
            self.cx = cx.min(self.lines[self.cy].chars().count());
            self.sel_anchor = None;
            self.dirty = true;
        }
        self.last_was_typing = false;
    }

    // ---- selection ----

    pub fn clear_selection(&mut self) {
        self.sel_anchor = None;
        self.last_was_typing = false;
    }

    /// Drops an anchor at the current cursor if none exists yet, so a following
    /// move extends the selection (used by Shift+arrows).
    pub fn ensure_anchor(&mut self) {
        if self.sel_anchor.is_none() {
            self.sel_anchor = Some((self.cy, self.cx));
        }
        self.last_was_typing = false;
    }

    pub fn select_all(&mut self) {
        self.sel_anchor = Some((0, 0));
        let last = self.lines.len() - 1;
        self.cy = last;
        self.cx = self.lines[last].chars().count();
        self.last_was_typing = false;
    }

    /// Returns the ordered selection bounds (start <= end).
    pub fn sel_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let a = self.sel_anchor?;
        let b = (self.cy, self.cx);
        if a == b {
            return None;
        }
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    pub fn selected_text(&self) -> Option<String> {
        let ((sr, sc), (er, ec)) = self.sel_range()?;
        if sr == er {
            let chars = self.line_chars(sr);
            Some(chars[sc..ec].iter().collect())
        } else {
            let mut s = String::new();
            let first = self.line_chars(sr);
            s.extend(first[sc..].iter());
            s.push('\n');
            for r in (sr + 1)..er {
                s.push_str(&self.lines[r]);
                s.push('\n');
            }
            let last = self.line_chars(er);
            s.extend(last[..ec].iter());
            Some(s)
        }
    }

    /// Deletes the selected text. Returns true if there was anything to delete.
    pub fn delete_selection(&mut self) -> bool {
        let Some(((sr, sc), (er, ec))) = self.sel_range() else {
            self.sel_anchor = None;
            return false;
        };
        if sr == er {
            let mut chars = self.line_chars(sr);
            chars.drain(sc..ec);
            self.lines[sr] = chars.into_iter().collect();
        } else {
            let first = self.line_chars(sr);
            let last = self.line_chars(er);
            let mut merged: String = first[..sc].iter().collect();
            let tail: String = last[ec..].iter().collect();
            merged.push_str(&tail);
            self.lines[sr] = merged;
            self.lines.drain((sr + 1)..=er);
        }
        self.cy = sr;
        self.cx = sc;
        self.sel_anchor = None;
        self.dirty = true;
        true
    }

    /// Inserts text at the cursor position (handles line breaks).
    pub fn insert_text(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\n' => self.insert_newline(),
                '\r' => {}
                _ => self.insert_char(ch),
            }
        }
    }

    // ---- cursor movement ----

    pub fn move_left(&mut self) {
        if self.cx > 0 {
            self.cx -= 1;
        } else if self.cy > 0 {
            self.cy -= 1;
            self.cx = self.cur_len();
        }
    }

    pub fn move_right(&mut self) {
        let len = self.cur_len();
        if self.cx < len {
            self.cx += 1;
        } else if self.cy + 1 < self.lines.len() {
            self.cy += 1;
            self.cx = 0;
        }
    }

    pub fn move_up(&mut self) {
        if self.cy > 0 {
            self.cy -= 1;
            self.clamp_cx();
        }
    }

    pub fn move_down(&mut self) {
        if self.cy + 1 < self.lines.len() {
            self.cy += 1;
            self.clamp_cx();
        }
    }

    pub fn move_home(&mut self) {
        self.cx = 0;
    }

    pub fn move_end(&mut self) {
        self.cx = self.cur_len();
    }

    /// Adjusts scrolling so the cursor stays visible within text_rows x text_cols.
    pub fn scroll(&mut self, text_rows: usize, text_cols: usize) {
        if self.cy < self.row_off {
            self.row_off = self.cy;
        }
        if text_rows > 0 && self.cy >= self.row_off + text_rows {
            self.row_off = self.cy - text_rows + 1;
        }
        if self.cx < self.col_off {
            self.col_off = self.cx;
        }
        if text_cols > 0 && self.cx >= self.col_off + text_cols {
            self.col_off = self.cx - text_cols + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> Buffer {
        let mut b = Buffer::new_named("t".to_string(), None);
        b.lines = text.split('\n').map(|s| s.to_string()).collect();
        b
    }

    #[test]
    fn select_all_covers_whole_text() {
        let mut b = buf("abc\ndef");
        b.select_all();
        assert_eq!(b.selected_text().as_deref(), Some("abc\ndef"));
        assert_eq!((b.cy, b.cx), (1, 3));
    }

    #[test]
    fn delete_selection_single_line() {
        let mut b = buf("hello");
        b.sel_anchor = Some((0, 1));
        b.cy = 0;
        b.cx = 4; // "ell" is selected
        assert!(b.delete_selection());
        assert_eq!(b.lines, vec!["ho".to_string()]);
        assert_eq!((b.cy, b.cx), (0, 1));
    }

    #[test]
    fn delete_selection_multi_line() {
        let mut b = buf("abc\ndef\nghi");
        b.sel_anchor = Some((0, 1));
        b.cy = 2;
        b.cx = 2; // from "a|bc" to "gh|i"
        assert!(b.delete_selection());
        assert_eq!(b.lines, vec!["ai".to_string()]);
    }

    #[test]
    fn replace_selection_by_insert() {
        let mut b = buf("abc\ndef");
        b.select_all();
        b.delete_selection();
        b.insert_text("X");
        assert_eq!(b.lines, vec!["X".to_string()]);
    }

    #[test]
    fn no_selection_returns_none() {
        let b = buf("abc");
        assert!(b.sel_range().is_none());
        assert!(b.selected_text().is_none());
    }

    #[test]
    fn undo_restores_previous_state() {
        let mut b = buf("abc");
        b.cx = 3;
        b.push_undo();
        b.insert_char('d'); // "abcd"
        assert_eq!(b.lines, vec!["abcd".to_string()]);
        b.undo();
        assert_eq!(b.lines, vec!["abc".to_string()]);
        assert_eq!((b.cy, b.cx), (0, 3));
    }

    #[test]
    fn undo_empty_history_is_noop() {
        let mut b = buf("abc");
        b.undo();
        assert_eq!(b.lines, vec!["abc".to_string()]);
    }

    #[test]
    fn insert_char_clamps_out_of_range_cursor() {
        let mut b = buf("ab");
        b.cx = 99; // past the end — must not panic
        b.insert_char('c');
        assert_eq!(b.lines, vec!["abc".to_string()]);
        assert_eq!(b.cx, 3);
    }

    fn temp_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("btix-test-{tag}-{nanos}"));
        p
    }

    #[test]
    fn from_path_rejects_invalid_utf8() {
        let path = temp_path("badutf8");
        fs::write(&path, [0xff, 0xfe, 0x41]).unwrap();
        let res = Buffer::from_path(path.clone());
        let _ = fs::remove_file(&path);
        assert!(res.is_err(), "invalid UTF-8 must not load as empty buffer");
    }

    #[test]
    fn from_path_missing_is_empty_buffer() {
        let path = temp_path("missing");
        let b = Buffer::from_path(path).unwrap();
        assert_eq!(b.lines, vec![String::new()]);
    }

    #[test]
    fn save_roundtrip_leaves_no_temp_file() {
        let path = temp_path("save");
        let mut b = Buffer::new_named("save".to_string(), Some(path.clone()));
        b.lines = vec!["hello".to_string(), "world".to_string()];
        b.dirty = true;
        b.save().unwrap();
        assert!(!b.dirty);
        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(written, "hello\nworld");
        let tmp = path
            .parent()
            .unwrap()
            .join(format!(".{}.btix-tmp", "save"));
        assert!(!tmp.exists(), "temp file must be renamed away");
        let _ = fs::remove_file(&path);
    }
}
