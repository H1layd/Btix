use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::buffer::Buffer;
use crate::config::{Action, Config, KeyChord};
use crate::highlight::Highlighter;

pub enum Mode {
    Normal,
    NewFile,
}

/// Live word-completion popup state.
pub struct Completion {
    pub items: Vec<String>,
    pub selected: usize,
    pub prefix_len: usize, // chars of the typed prefix, replaced on accept
}

/// Shortest prefix that opens the completion popup.
const MIN_PREFIX: usize = 2;

pub struct Editor {
    pub buffers: Vec<Buffer>,
    pub active: usize,
    pub hl: Highlighter,
    pub dir: PathBuf,
    pub mode: Mode,
    pub prompt: String,
    pub clipboard: String,
    pub config: Config,
    pub status: String,
    pub suggestion: String,
    pub completion: Option<Completion>,
    pub needs_check: bool,
    pub quit: bool,
}

impl Editor {
    pub fn new(dir: PathBuf, files: Vec<PathBuf>, config: Config) -> Self {
        let mut buffers: Vec<Buffer> = files
            .into_iter()
            .filter_map(|p| Buffer::from_path(p).ok())
            .collect();
        if buffers.is_empty() {
            buffers.push(Buffer::new_named("untitled".to_string(), None));
        }
        let hl = Highlighter::new(config.theme.clone());
        Editor {
            buffers,
            active: 0,
            hl,
            dir,
            mode: Mode::Normal,
            prompt: String::new(),
            clipboard: String::new(),
            config,
            status: String::new(),
            suggestion: String::new(),
            completion: None,
            needs_check: false,
            quit: false,
        }
    }

    /// Records state before a change. Consecutive typed characters are
    /// coalesced into a single undo step (typing=true).
    fn record_undo(&mut self, typing: bool) {
        // Any edit invalidates the last syntax-check result and schedules a
        // fresh auto-check once the user pauses.
        self.needs_check = true;
        let b = &mut self.buffers[self.active];
        b.error_line = None;
        b.error_col = None;
        if typing && b.last_was_typing {
            return;
        }
        b.push_undo();
        b.last_was_typing = typing;
    }

    pub fn buf(&mut self) -> &mut Buffer {
        &mut self.buffers[self.active]
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::NewFile => self.handle_prompt(key),
        }
    }

    /// Looks up the action bound to this key event, if any.
    fn match_action(&self, key: &KeyEvent) -> Option<Action> {
        let chord = KeyChord::from_event(key)?;
        self.config
            .keybinds
            .iter()
            .find(|(c, _)| *c == chord)
            .map(|(_, a)| *a)
    }

    fn dispatch(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::Save => self.save_active(),
            Action::Undo => self.buf().undo(),
            Action::SelectAll => self.buf().select_all(),
            Action::Copy => self.copy(),
            Action::Cut => self.cut(),
            Action::Paste => self.paste_clipboard(),
            Action::NewFile => self.start_new_file(),
            Action::PrevTab => self.prev_tab(),
            Action::NextTab => self.next_tab(),
            Action::Check => self.run_check(true),
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) {
        // A status message lives until the next keypress, then clears.
        self.status.clear();
        self.suggestion.clear();

        // While the completion popup is open it captures its control keys
        // (navigate / accept / dismiss) before anything else.
        if self.completion.is_some() && self.handle_completion_key(&key) {
            return;
        }

        // Configurable bindings take precedence over default movement/editing.
        if let Some(action) = self.match_action(&key) {
            self.completion = None;
            self.dispatch(action);
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        // Shift+move extends a selection; a plain move clears it.
        let sel_edit = |b: &mut Buffer| {
            if shift {
                b.ensure_anchor();
            } else {
                b.clear_selection();
            }
        };

        match key.code {
            KeyCode::Left => {
                let b = self.buf();
                sel_edit(b);
                b.move_left();
            }
            KeyCode::Right => {
                let b = self.buf();
                sel_edit(b);
                b.move_right();
            }
            KeyCode::Up => {
                let b = self.buf();
                sel_edit(b);
                b.move_up();
            }
            KeyCode::Down => {
                let b = self.buf();
                sel_edit(b);
                b.move_down();
            }
            KeyCode::Home => {
                let b = self.buf();
                sel_edit(b);
                b.move_home();
            }
            KeyCode::End => {
                let b = self.buf();
                sel_edit(b);
                b.move_end();
            }

            KeyCode::Enter => {
                self.record_undo(false);
                let b = self.buf();
                b.delete_selection();
                b.insert_newline();
            }
            KeyCode::Backspace => {
                self.record_undo(false);
                let b = self.buf();
                if !b.delete_selection() {
                    b.backspace();
                }
            }
            KeyCode::Delete => {
                self.record_undo(false);
                let b = self.buf();
                if !b.delete_selection() {
                    b.delete_forward();
                }
            }
            KeyCode::Tab => {
                self.record_undo(false);
                let w = self.config.tab_width;
                let b = self.buf();
                b.delete_selection();
                for _ in 0..w {
                    b.insert_char(' ');
                }
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                // Replacing a selection is its own undo step; plain typing is coalesced.
                let has_sel = self.buf().sel_range().is_some();
                self.record_undo(!has_sel);
                let b = self.buf();
                b.delete_selection();
                b.insert_char(c);
                // In word-level mode a space ends the current typing run, so the
                // next character starts a fresh undo group.
                if !has_sel && self.config.undo_stop_at_spaces && c == ' ' {
                    self.buf().last_was_typing = false;
                }
            }
            _ => {}
        }

        // Typing and Backspace refine the popup; anything else dismisses it.
        match key.code {
            KeyCode::Char(_) if !ctrl && !alt => self.update_completion(),
            KeyCode::Backspace => self.update_completion(),
            _ => self.completion = None,
        }
    }

    /// Handles a key while the completion popup is open. Returns true if the key
    /// was consumed (navigation, accept, or dismiss). Only plain keys with no
    /// modifiers steer the popup; modified keys fall through to normal handling.
    fn handle_completion_key(&mut self, key: &KeyEvent) -> bool {
        if !key.modifiers.is_empty() {
            return false;
        }
        let Some(c) = self.completion.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Up => {
                c.selected = (c.selected + c.items.len() - 1) % c.items.len();
                true
            }
            KeyCode::Down => {
                c.selected = (c.selected + 1) % c.items.len();
                true
            }
            KeyCode::Tab | KeyCode::Enter => {
                self.accept_completion();
                true
            }
            KeyCode::Esc => {
                self.completion = None;
                true
            }
            _ => false,
        }
    }

    /// Recomputes the popup from the word prefix before the cursor.
    fn update_completion(&mut self) {
        if !self.config.autocomplete {
            self.completion = None;
            return;
        }
        let b = &self.buffers[self.active];
        let ext = b.extension();
        let line = &b.lines[b.cy];
        let prefix = crate::complete::word_prefix(line, b.cx);
        if prefix.chars().count() < MIN_PREFIX {
            self.completion = None;
            return;
        }
        let items = crate::complete::candidates(&ext, &b.lines, &prefix);
        if items.is_empty() {
            self.completion = None;
        } else {
            self.completion = Some(Completion {
                items,
                selected: 0,
                prefix_len: prefix.chars().count(),
            });
        }
    }

    /// Replaces the typed prefix with the selected candidate.
    fn accept_completion(&mut self) {
        let Some(c) = self.completion.take() else {
            return;
        };
        let Some(item) = c.items.get(c.selected).cloned() else {
            return;
        };
        self.record_undo(false);
        let b = self.buf();
        for _ in 0..c.prefix_len {
            b.backspace();
        }
        b.insert_text(&item);
    }

    fn copy(&mut self) {
        if let Some(text) = self.buffers[self.active].selected_text() {
            self.clipboard = text.clone();
            set_system_clipboard(&text);
        }
    }

    fn cut(&mut self) {
        if let Some(text) = self.buffers[self.active].selected_text() {
            self.clipboard = text.clone();
            set_system_clipboard(&text);
            self.record_undo(false);
            self.buffers[self.active].delete_selection();
        }
    }

    fn paste_clipboard(&mut self) {
        let mut text = get_system_clipboard();
        if text.is_empty() {
            text = self.clipboard.clone();
        }
        if !text.is_empty() {
            self.paste_text(&text);
        }
    }

    /// Inserts text (from Ctrl+V or the terminal's bracketed paste).
    pub fn paste_text(&mut self, text: &str) {
        self.record_undo(false);
        let b = &mut self.buffers[self.active];
        b.delete_selection();
        b.insert_text(text);
    }

    fn handle_prompt(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.prompt.clear();
            }
            KeyCode::Enter => self.confirm_new_file(),
            KeyCode::Backspace => {
                self.prompt.pop();
            }
            KeyCode::Char(c) => self.prompt.push(c),
            _ => {}
        }
    }

    fn save_active(&mut self) {
        let _ = self.buffers[self.active].save();
    }

    /// Runs the auto syntax check if it is enabled and the buffer changed since
    /// the last check. Called when the user pauses typing.
    pub fn auto_check(&mut self) {
        if self.config.auto_check && self.needs_check {
            self.run_check(false);
        }
    }

    /// Runs a syntax check on the active buffer and reports the result in the
    /// status bar, flagging the offending line. A manual check also explains
    /// when no checker applies, jumps the cursor to the error, and confirms
    /// "Syntax OK"; the auto check stays quiet in those cases to avoid noise.
    fn run_check(&mut self, manual: bool) {
        self.needs_check = false;

        let ext = self.buffers[self.active].extension();
        if ext.is_empty() {
            if manual {
                self.status = "No file extension to check".to_string();
            }
            return;
        }
        let cmd = self
            .config
            .checkers
            .iter()
            .find(|(e, _)| *e == ext)
            .map(|(_, c)| c.clone())
            .or_else(|| crate::check::builtin(&ext));
        let Some(cmd) = cmd else {
            if manual {
                self.status = format!("No syntax checker for .{ext}");
            }
            return;
        };

        let content = self.buffers[self.active].lines.join("\n");
        let dir = self.buffers[self.active]
            .path
            .as_ref()
            .and_then(|p| p.parent().map(|x| x.to_path_buf()))
            .unwrap_or_else(|| self.dir.clone());

        let res = crate::check::run(&cmd, &ext, &content, &dir);
        let b = &mut self.buffers[self.active];
        b.error_line = res.error_line;
        b.error_col = res.error_col;
        // Prefer the linter's own wording ("what exactly is wrong") over our
        // generic headline. The suggestion is the tool's own "did you mean".
        self.suggestion = res.suggestion.clone().unwrap_or_default();
        match res.error_line {
            Some(idx) => {
                // Only a manual check moves the cursor — jumping while the user
                // is typing would be disruptive.
                if manual {
                    b.cy = idx.min(b.lines.len().saturating_sub(1));
                    b.cx = res.error_col.unwrap_or(0).min(b.cur_len());
                    b.clear_selection();
                }
                self.status = res.detail.unwrap_or(res.message);
            }
            None => {
                if manual {
                    self.status = res.detail.unwrap_or(res.message);
                } else {
                    self.status.clear();
                    self.suggestion.clear();
                }
            }
        }
    }

    fn start_new_file(&mut self) {
        self.mode = Mode::NewFile;
        self.prompt.clear();
    }

    fn confirm_new_file(&mut self) {
        let name = self.prompt.trim().to_string();
        self.mode = Mode::Normal;
        self.prompt.clear();
        // Keep new files inside the current folder: reject empty names, path
        // separators, and parent/current-dir entries so a typed name cannot
        // create or clobber a file elsewhere on disk.
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name == "."
            || name == ".."
        {
            return;
        }
        let path = self.dir.join(&name);
        // Create an empty file on disk so it shows up as a tab right away.
        if fs::write(&path, "").is_err() {
            return;
        }
        let buf = Buffer::new_named(name, Some(path));
        self.buffers.push(buf);
        self.active = self.buffers.len() - 1;
    }

    fn next_tab(&mut self) {
        if self.buffers.len() > 1 {
            self.active = (self.active + 1) % self.buffers.len();
        }
    }

    fn prev_tab(&mut self) {
        if self.buffers.len() > 1 {
            self.active = (self.active + self.buffers.len() - 1) % self.buffers.len();
        }
    }
}

/// Copies text to the macOS system clipboard via pbcopy.
/// Absolute path so a `pbcopy` planted earlier in $PATH can't be run instead.
fn set_system_clipboard(text: &str) {
    if let Ok(mut child) = Command::new("/usr/bin/pbcopy").stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

/// Reads the macOS system clipboard via pbpaste.
/// Absolute path so a `pbpaste` planted earlier in $PATH can't be run instead.
fn get_system_clipboard() -> String {
    Command::new("/usr/bin/pbpaste")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}
