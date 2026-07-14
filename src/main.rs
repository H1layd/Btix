mod buffer;
mod check;
mod complete;
mod config;
mod editor;
mod highlight;
mod ui;
mod update;

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, queue};

use editor::Editor;

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();

    // Config-creation flag — runs and exits.
    if args.iter().any(|a| a == "--get-config") {
        return config::create_default();
    }

    // Update-check flag — runs and exits.
    if args.iter().any(|a| a == "--check-update") {
        return update::check();
    }

    // The first non-flag argument is a path to a file/folder.
    let arg = args.into_iter().find(|a| !a.starts_with("--"));
    let (dir, focus): (PathBuf, Option<PathBuf>) = match arg {
        Some(a) => {
            let p = PathBuf::from(&a);
            if p.is_dir() {
                (p, None)
            } else if p.is_file() {
                let parent = p
                    .parent()
                    .map(|x| x.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("."));
                (parent, Some(p.canonicalize().unwrap_or(p)))
            } else {
                // Nonexistent path: treat it as a new file in the current folder.
                let cwd = env::current_dir()?;
                (cwd, Some(PathBuf::from(&a)))
            }
        }
        None => (env::current_dir()?, None),
    };

    let mut files = scan_dir(&dir);
    if let Some(f) = &focus {
        let canon = f.canonicalize().ok();
        let already = files.iter().any(|p| p.canonicalize().ok() == canon);
        if !already {
            files.insert(0, f.clone());
        }
    }

    let cfg = config::load();
    let mut ed = Editor::new(dir, files, cfg);

    // If a focus file was given, make it the active tab.
    if let Some(f) = &focus {
        let canon = f.canonicalize().ok();
        if let Some(idx) = ed
            .buffers
            .iter()
            .position(|b| b.path.as_ref().and_then(|p| p.canonicalize().ok()) == canon)
        {
            ed.active = idx;
        }
    }

    run(&mut ed)
}

fn run(ed: &mut Editor) -> io::Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;

    // Enable the enhanced keyboard protocol when possible — this lets the
    // terminal distinguish Ctrl+Shift+<key> combinations.
    let enhanced = terminal::supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }

    let result = event_loop(ed, &mut stdout);

    if enhanced {
        let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    }
    execute!(stdout, DisableBracketedPaste, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    result
}

/// How long the user must pause typing before the auto syntax check fires.
const IDLE_MS: u64 = 600;

fn event_loop<W: Write>(ed: &mut Editor, out: &mut W) -> io::Result<()> {
    let (mut cols, mut rows) = terminal::size()?;
    loop {
        queue!(
            out,
            terminal::Clear(terminal::ClearType::All)
        )?;
        ui::draw(out, ed, cols, rows)?;

        // Wait for input, but wake up after IDLE_MS so a debounced auto-check
        // can run once the user stops typing. Continuous typing keeps resetting
        // the wait, so the check only fires during a pause.
        if event::poll(Duration::from_millis(IDLE_MS))? {
            match event::read()? {
                Event::Key(key) => ed.handle_key(key),
                Event::Paste(text) => ed.paste_text(&text),
                Event::Resize(c, r) => {
                    cols = c;
                    rows = r;
                }
                _ => {}
            }
        } else {
            ed.auto_check();
        }
        if ed.quit {
            break;
        }
    }
    Ok(())
}

fn scan_dir(dir: &PathBuf) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if meta.len() > 5_000_000 {
                    continue;
                }
            }
            if is_text(&p) {
                v.push(p);
            }
        }
    }
    v.sort();
    v
}

/// Rough heuristic: a file is considered text if it has no NUL bytes near the start.
fn is_text(p: &PathBuf) -> bool {
    match fs::read(p) {
        Ok(bytes) => {
            let head = &bytes[..bytes.len().min(8192)];
            !head.contains(&0)
        }
        Err(_) => false,
    }
}
