use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// An editor command that can be bound to a key chord in the config.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Save,
    Undo,
    SelectAll,
    Copy,
    Cut,
    Paste,
    NewFile,
    PrevTab,
    NextTab,
    Check,
}

/// The non-modifier part of a key chord.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChordKey {
    Char(char),
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Tab,
    Enter,
    Backspace,
    Delete,
    Esc,
    Space,
    PageUp,
    PageDown,
    F(u8),
}

/// A full key combination: modifiers plus a key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: ChordKey,
}

impl KeyChord {
    /// Builds a chord from a live key event so it can be matched against bindings.
    /// Char keys are lowercased and Shift is dropped for them, because terminals
    /// already deliver the shifted character and report Shift inconsistently.
    pub fn from_event(ev: &KeyEvent) -> Option<KeyChord> {
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        let mut shift = ev.modifiers.contains(KeyModifiers::SHIFT);
        let alt = ev.modifiers.contains(KeyModifiers::ALT);
        let key = match ev.code {
            KeyCode::Char(' ') => ChordKey::Space,
            KeyCode::Char(c) => {
                shift = false;
                ChordKey::Char(c.to_ascii_lowercase())
            }
            KeyCode::Left => ChordKey::Left,
            KeyCode::Right => ChordKey::Right,
            KeyCode::Up => ChordKey::Up,
            KeyCode::Down => ChordKey::Down,
            KeyCode::Home => ChordKey::Home,
            KeyCode::End => ChordKey::End,
            KeyCode::Tab => ChordKey::Tab,
            KeyCode::BackTab => {
                shift = true;
                ChordKey::Tab
            }
            KeyCode::Enter => ChordKey::Enter,
            KeyCode::Backspace => ChordKey::Backspace,
            KeyCode::Delete => ChordKey::Delete,
            KeyCode::Esc => ChordKey::Esc,
            KeyCode::PageUp => ChordKey::PageUp,
            KeyCode::PageDown => ChordKey::PageDown,
            KeyCode::F(n) => ChordKey::F(n),
            _ => return None,
        };
        Some(KeyChord {
            ctrl,
            shift,
            alt,
            key,
        })
    }
}

/// Editor settings, read from the config file.
pub struct Config {
    pub theme: String,
    pub tab_width: usize,
    pub show_line_numbers: bool,
    pub selection_bg: (u8, u8, u8),
    pub line_number_fg: (u8, u8, u8),

    // Bars: existence and colors.
    pub show_top_bar: bool,
    pub show_bottom_bar: bool,
    pub bar_bg: (u8, u8, u8),
    pub bar_fg: (u8, u8, u8),
    pub active_tab_bg: (u8, u8, u8),
    pub active_tab_fg: (u8, u8, u8),

    // When true, Ctrl+Z stops at spaces (word-level undo). When false, one
    // undo step covers the whole typed run regardless of spaces.
    pub undo_stop_at_spaces: bool,

    // When true, the syntax check runs automatically after a short pause in
    // typing (in addition to the manual key binding).
    pub auto_check: bool,

    // When true, a word-completion popup appears as you type (keywords/builtins
    // plus identifiers already in the file).
    pub autocomplete: bool,

    // Key bindings: a chord may map to an action; several chords can share one.
    pub keybinds: Vec<(KeyChord, Action)>,

    // Syntax-checker command overrides per extension (e.g. "py" -> ["ruff","check"]).
    // Looked up before the built-in table.
    pub checkers: Vec<(String, Vec<String>)>,
}

/// Default key bindings as (config key, default value, action). Single source
/// of truth shared by Config::default() and the generated template.
const DEFAULT_KEYBINDS: &[(&str, &str, Action)] = &[
    ("key_quit", "ctrl+q", Action::Quit),
    ("key_save", "ctrl+s", Action::Save),
    ("key_undo", "ctrl+z", Action::Undo),
    ("key_select_all", "ctrl+a", Action::SelectAll),
    ("key_copy", "ctrl+c", Action::Copy),
    ("key_cut", "ctrl+x", Action::Cut),
    ("key_paste", "ctrl+v", Action::Paste),
    ("key_new_file", "ctrl+n, ctrl+shift+w", Action::NewFile),
    ("key_prev_tab", "ctrl+left, ctrl+shift+left", Action::PrevTab),
    ("key_next_tab", "ctrl+right, ctrl+shift+right", Action::NextTab),
    ("key_check", "ctrl+e", Action::Check),
];

fn default_keybinds() -> Vec<(KeyChord, Action)> {
    let mut binds = Vec::new();
    for (_, value, action) in DEFAULT_KEYBINDS {
        for chord in parse_chords(value) {
            binds.push((chord, *action));
        }
    }
    binds
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: "base16-ocean.dark".to_string(),
            tab_width: 4,
            show_line_numbers: true,
            selection_bg: (60, 80, 120),
            line_number_fg: (100, 110, 130),
            show_top_bar: true,
            show_bottom_bar: true,
            bar_bg: (45, 55, 72),
            bar_fg: (226, 232, 240),
            active_tab_bg: (66, 153, 225),
            active_tab_fg: (26, 32, 44),
            undo_stop_at_spaces: false,
            auto_check: true,
            autocomplete: true,
            keybinds: default_keybinds(),
            checkers: Vec::new(),
        }
    }
}

/// Config path: $HOME/.config/btix/config.toml
pub fn config_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config/btix/config.toml")
}

/// Returns the action a `key_*` config key configures, if any.
fn action_for_key(key: &str) -> Option<Action> {
    DEFAULT_KEYBINDS
        .iter()
        .find(|(name, _, _)| *name == key)
        .map(|(_, _, action)| *action)
}

/// Loads the config, falling back to defaults for missing keys.
pub fn load() -> Config {
    let mut cfg = Config::default();
    let path = config_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return cfg;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim().trim_matches('"');

        if let Some(action) = action_for_key(key) {
            // Replace all default bindings for this action with the configured ones.
            cfg.keybinds.retain(|(_, a)| *a != action);
            for chord in parse_chords(val) {
                cfg.keybinds.push((chord, action));
            }
            continue;
        }

        // Per-extension syntax-checker override: "checker_<ext> = cmd args".
        if let Some(ext) = key.strip_prefix("checker_") {
            let words: Vec<String> = val.split_whitespace().map(|w| w.to_string()).collect();
            cfg.checkers.retain(|(e, _)| e != ext);
            if !words.is_empty() {
                cfg.checkers.push((ext.to_string(), words));
            }
            continue;
        }

        match key {
            "theme" => cfg.theme = val.to_string(),
            "tab_width" => {
                if let Ok(n) = val.parse::<usize>() {
                    cfg.tab_width = n.clamp(1, 16);
                }
            }
            "show_line_numbers" => cfg.show_line_numbers = val == "true",
            "selection_bg" => {
                if let Some(c) = parse_color(val) {
                    cfg.selection_bg = c;
                }
            }
            "line_number_fg" => {
                if let Some(c) = parse_color(val) {
                    cfg.line_number_fg = c;
                }
            }
            "show_top_bar" => cfg.show_top_bar = val == "true",
            "show_bottom_bar" => cfg.show_bottom_bar = val == "true",
            "bar_bg" => {
                if let Some(c) = parse_color(val) {
                    cfg.bar_bg = c;
                }
            }
            "bar_fg" => {
                if let Some(c) = parse_color(val) {
                    cfg.bar_fg = c;
                }
            }
            "active_tab_bg" => {
                if let Some(c) = parse_color(val) {
                    cfg.active_tab_bg = c;
                }
            }
            "active_tab_fg" => {
                if let Some(c) = parse_color(val) {
                    cfg.active_tab_fg = c;
                }
            }
            "undo_stop_at_spaces" => cfg.undo_stop_at_spaces = val == "true",
            "auto_check" => cfg.auto_check = val == "true",
            "autocomplete" => cfg.autocomplete = val == "true",
            _ => {}
        }
    }
    cfg
}

/// Creates a config with default settings. Invoked by the --get-config flag.
pub fn create_default() -> io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        println!("Config already exists: {}", path.display());
        return Ok(());
    }
    fs::write(&path, template())?;
    println!("Config created: {}", path.display());
    Ok(())
}

fn template() -> String {
    let d = Config::default();
    let mut keys = String::new();
    for (name, value, _) in DEFAULT_KEYBINDS {
        keys.push_str(&format!("{} = \"{}\"\n", name, value));
    }
    format!(
        "# btix editor configuration\n\
         # Restart the program after editing.\n\
         \n\
         # Syntax highlighting theme.\n\
         # Available: base16-ocean.dark, base16-eighties.dark, base16-mocha.dark,\n\
         #            base16-ocean.light, InspiredGitHub, Solarized (dark), Solarized (light)\n\
         theme = \"{theme}\"\n\
         \n\
         # How many spaces Tab inserts (1..16).\n\
         tab_width = {tab_width}\n\
         \n\
         # Show line numbers on the left (true/false).\n\
         show_line_numbers = {show_line_numbers}\n\
         \n\
         # Selection background color, \"r,g,b\" or \"#rrggbb\".\n\
         selection_bg = \"{sel_r},{sel_g},{sel_b}\"\n\
         \n\
         # Line number color, \"r,g,b\" or \"#rrggbb\".\n\
         line_number_fg = \"{ln_r},{ln_g},{ln_b}\"\n\
         \n\
         # Top bar (centered file name): show it or not.\n\
         show_top_bar = {show_top_bar}\n\
         \n\
         # Bottom bar (file tabs): show it or not.\n\
         show_bottom_bar = {show_bottom_bar}\n\
         \n\
         # Bar background / foreground color.\n\
         bar_bg = \"{bar_bg_r},{bar_bg_g},{bar_bg_b}\"\n\
         bar_fg = \"{bar_fg_r},{bar_fg_g},{bar_fg_b}\"\n\
         \n\
         # Active tab background / foreground color.\n\
         active_tab_bg = \"{atb_r},{atb_g},{atb_b}\"\n\
         active_tab_fg = \"{atf_r},{atf_g},{atf_b}\"\n\
         \n\
         # Undo granularity (Ctrl+Z).\n\
         # true  = stop at spaces, so undo removes one word at a time.\n\
         # false = one undo step covers the whole typed run.\n\
         undo_stop_at_spaces = {undo_stop_at_spaces}\n\
         \n\
         # Key bindings. Modifiers: ctrl, shift, alt. Keys: letters, digits,\n\
         # left/right/up/down, home, end, tab, enter, backspace, delete, esc,\n\
         # space, pageup, pagedown, f1..f12. Separate alternatives with commas.\n\
         # Note: many terminals (e.g. macOS Terminal.app) cannot send ctrl+shift\n\
         # combos; the single-ctrl alternatives are there as a fallback.\n\
         {keys}\
         \n\
         # Run the syntax check automatically after a short pause in typing.\n\
         auto_check = {auto_check}\n\
         \n\
         # Word-completion popup as you type (Tab or Enter accepts, arrows pick,\n\
         # Esc dismisses). Candidates are language keywords/builtins plus words\n\
         # already present in the file.\n\
         autocomplete = {autocomplete}\n\
         \n\
         # Syntax checking (the key_check action runs it on the current file).\n\
         # Built-in checkers exist for py, js, ts, rb, php, sh, lua, go, c, cpp,\n\
         # rs, json, yaml and more; each needs that language's tool installed.\n\
         # Override or add one per extension; the file is appended as the last arg:\n\
         #   checker_py = ruff check\n\
         #   checker_js = eslint\n",
        theme = d.theme,
        tab_width = d.tab_width,
        show_line_numbers = d.show_line_numbers,
        sel_r = d.selection_bg.0,
        sel_g = d.selection_bg.1,
        sel_b = d.selection_bg.2,
        ln_r = d.line_number_fg.0,
        ln_g = d.line_number_fg.1,
        ln_b = d.line_number_fg.2,
        show_top_bar = d.show_top_bar,
        show_bottom_bar = d.show_bottom_bar,
        bar_bg_r = d.bar_bg.0,
        bar_bg_g = d.bar_bg.1,
        bar_bg_b = d.bar_bg.2,
        bar_fg_r = d.bar_fg.0,
        bar_fg_g = d.bar_fg.1,
        bar_fg_b = d.bar_fg.2,
        atb_r = d.active_tab_bg.0,
        atb_g = d.active_tab_bg.1,
        atb_b = d.active_tab_bg.2,
        atf_r = d.active_tab_fg.0,
        atf_g = d.active_tab_fg.1,
        atf_b = d.active_tab_fg.2,
        undo_stop_at_spaces = d.undo_stop_at_spaces,
        auto_check = d.auto_check,
        autocomplete = d.autocomplete,
        keys = keys,
    )
}

/// Parses a comma-separated list of chords, skipping any that fail to parse.
fn parse_chords(s: &str) -> Vec<KeyChord> {
    s.split(',')
        .filter_map(|part| parse_chord(part.trim()))
        .collect()
}

/// Parses one chord like "ctrl+shift+left" into a KeyChord.
fn parse_chord(s: &str) -> Option<KeyChord> {
    if s.is_empty() {
        return None;
    }
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut key = None;
    for tok in s.split('+') {
        let tok = tok.trim().to_ascii_lowercase();
        match tok.as_str() {
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" | "option" | "meta" => alt = true,
            other => key = parse_key(other),
        }
    }
    let mut key = key?;
    // For a single character, fold case and treat Shift as already applied.
    if let ChordKey::Char(c) = key {
        key = ChordKey::Char(c.to_ascii_lowercase());
        shift = false;
    }
    Some(KeyChord {
        ctrl,
        shift,
        alt,
        key,
    })
}

fn parse_key(s: &str) -> Option<ChordKey> {
    let key = match s {
        "left" => ChordKey::Left,
        "right" => ChordKey::Right,
        "up" => ChordKey::Up,
        "down" => ChordKey::Down,
        "home" => ChordKey::Home,
        "end" => ChordKey::End,
        "tab" => ChordKey::Tab,
        "enter" | "return" => ChordKey::Enter,
        "backspace" => ChordKey::Backspace,
        "delete" | "del" => ChordKey::Delete,
        "esc" | "escape" => ChordKey::Esc,
        "space" => ChordKey::Space,
        "pageup" => ChordKey::PageUp,
        "pagedown" => ChordKey::PageDown,
        _ => {
            if let Some(num) = s.strip_prefix('f') {
                if let Ok(n) = num.parse::<u8>() {
                    if (1..=12).contains(&n) {
                        return Some(ChordKey::F(n));
                    }
                }
                return None;
            }
            let mut chars = s.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            ChordKey::Char(c.to_ascii_lowercase())
        }
    };
    Some(key)
}

fn parse_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some((r, g, b));
        }
        return None;
    }
    let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
    if parts.len() == 3 {
        let r = parts[0].parse().ok()?;
        let g = parts[1].parse().ok()?;
        let b = parts[2].parse().ok()?;
        return Some((r, g, b));
    }
    None
}
