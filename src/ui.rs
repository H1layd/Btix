use std::io::{self, Write};

use crossterm::cursor::MoveTo;
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, queue};

use crate::editor::{Editor, Mode};

const GUTTER_FG: Color = Color::Rgb { r: 100, g: 110, b: 130 };

fn rgb(c: (u8, u8, u8)) -> Color {
    Color::Rgb {
        r: c.0,
        g: c.1,
        b: c.2,
    }
}

pub fn draw<W: Write>(out: &mut W, ed: &mut Editor, cols: u16, rows: u16) -> io::Result<()> {
    let cols = cols as usize;
    let rows = rows as usize;
    if rows < 3 || cols < 4 {
        return Ok(());
    }

    // Bar layout depends on config; the prompt or a status message forces a
    // bottom row even when the bottom bar is disabled, and a suggestion adds a
    // second row beneath the error message.
    let in_prompt = matches!(ed.mode, Mode::NewFile);
    let has_status = !ed.status.is_empty();
    let has_suggestion = has_status && !ed.suggestion.is_empty();
    let top = usize::from(ed.config.show_top_bar);
    let bottom = if in_prompt {
        1
    } else if has_status {
        1 + usize::from(has_suggestion)
    } else {
        usize::from(ed.config.show_bottom_bar)
    };
    let text_rows = rows.saturating_sub(top + bottom);

    // Gutter geometry depends on the active buffer's line count and the setting.
    let line_count = ed.buffers[ed.active].lines.len();
    let (num_width, gutter) = if ed.config.show_line_numbers {
        let nw = line_count.to_string().len().max(3);
        (nw, nw + 1) // number + space
    } else {
        (0, 0)
    };
    let text_cols = cols.saturating_sub(gutter);

    // Scroll to keep the cursor visible.
    ed.buffers[ed.active].scroll(text_rows, text_cols);

    queue!(out, cursor::Hide)?;

    if top == 1 {
        draw_top_bar(out, ed, cols)?;
    }
    let error_line = ed.buffers[ed.active].error_line;
    let error_col = ed.buffers[ed.active].error_col;
    draw_text(
        out, ed, top, gutter, num_width, text_cols, text_rows, error_line, error_col,
    )?;
    if bottom > 0 {
        draw_bottom_bar(out, ed, cols, rows, bottom)?;
    }

    // Cursor. Saturating math: even if scroll state is briefly inconsistent,
    // this can never underflow and panic.
    let b = &ed.buffers[ed.active];
    let cur_x = gutter + b.cx.saturating_sub(b.col_off);
    let cur_y = top + b.cy.saturating_sub(b.row_off);

    // Completion popup overlays the text, anchored at the cursor. Drawn before
    // the final cursor placement so the caret stays on the typed prefix.
    draw_completion(out, ed, cur_x, cur_y, cols, rows)?;

    queue!(out, MoveTo(cur_x as u16, cur_y as u16), cursor::Show)?;

    out.flush()
}

/// Replaces control characters so untrusted file names or typed input cannot
/// inject terminal escape sequences when drawn into a bar.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { '·' } else { c })
        .collect()
}

fn pad(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.chars().take(width).collect()
    } else {
        let mut r = s.to_string();
        r.extend(std::iter::repeat(' ').take(width - len));
        r
    }
}

fn draw_top_bar<W: Write>(out: &mut W, ed: &Editor, cols: usize) -> io::Result<()> {
    let b = &ed.buffers[ed.active];
    let mark = if b.dirty { " ●" } else { "" };
    let title = format!("{}{}", sanitize(&b.name), mark);
    let tw = title.chars().count();
    let left_pad = if tw < cols { (cols - tw) / 2 } else { 0 };
    let line = format!("{}{}", " ".repeat(left_pad), title);
    queue!(
        out,
        MoveTo(0, 0),
        Clear(ClearType::CurrentLine),
        SetForegroundColor(rgb(ed.config.bar_fg)),
        SetAttribute(Attribute::Bold),
        Print(pad(&line, cols)),
        SetAttribute(Attribute::Reset),
        ResetColor
    )
}

fn draw_text<W: Write>(
    out: &mut W,
    ed: &Editor,
    top: usize,
    gutter: usize,
    num_width: usize,
    text_cols: usize,
    text_rows: usize,
    error_line: Option<usize>,
    error_col: Option<usize>,
) -> io::Result<()> {
    let b = &ed.buffers[ed.active];
    let gutter_fg = rgb(ed.config.line_number_fg);
    let selection_bg = rgb(ed.config.selection_bg);
    // Strong red for the exact offending column; softer red wash for the whole
    // line when the checker only gave us a line number.
    let error_bg_strong = Color::Rgb { r: 150, g: 40, b: 40 };
    let error_bg_soft = Color::Rgb { r: 70, g: 28, b: 28 };
    let last_visible = b.row_off + text_rows;
    let spans = ed
        .hl
        .highlight(&b.lines, &b.extension(), b.row_off, last_visible.saturating_sub(1));

    for r in 0..text_rows {
        let file_row = b.row_off + r;
        let screen_y = (top + r) as u16;
        queue!(out, MoveTo(0, screen_y), Clear(ClearType::CurrentLine))?;

        if file_row >= b.lines.len() {
            queue!(out, SetForegroundColor(gutter_fg), Print("~"), ResetColor)?;
            continue;
        }

        // Line number (if enabled in the config). The line flagged by the
        // syntax checker is shown in red.
        if num_width > 0 {
            let num = format!("{:>width$} ", file_row + 1, width = num_width);
            let fg = if Some(file_row) == error_line {
                Color::Rgb { r: 229, g: 62, b: 62 }
            } else {
                gutter_fg
            };
            queue!(out, SetForegroundColor(fg), Print(num), ResetColor)?;
        }

        // Highlighted text with horizontal scrolling, selection, and the syntax
        // error highlight. Selection wins over the error wash where they overlap.
        let sel = b.sel_range();
        let is_err_line = Some(file_row) == error_line;
        let Some(line_spans) = spans.get(r) else {
            continue;
        };
        let mut col = 0usize;
        let mut cur_bg = Color::Reset;
        queue!(out, SetBackgroundColor(Color::Reset))?;
        'line: for (color, text) in line_spans {
            queue!(out, SetForegroundColor(*color))?;
            for ch in text.chars() {
                if col >= b.col_off + text_cols {
                    break 'line;
                }
                if col >= b.col_off {
                    let desired = if in_selection(sel, file_row, col) {
                        selection_bg
                    } else if is_err_line {
                        match error_col {
                            Some(ec) if ec == col => error_bg_strong,
                            _ => error_bg_soft,
                        }
                    } else {
                        Color::Reset
                    };
                    if desired != cur_bg {
                        queue!(out, SetBackgroundColor(desired))?;
                        cur_bg = desired;
                    }
                    // Never write raw control characters to the terminal: a file
                    // could contain escape sequences that would otherwise be
                    // interpreted by the terminal (title spoofing, etc.).
                    let safe = if ch.is_control() { '·' } else { ch };
                    queue!(out, Print(safe))?;
                }
                col += 1;
            }
        }
        let _ = gutter;
        queue!(out, SetBackgroundColor(Color::Reset), ResetColor)?;
    }
    Ok(())
}

/// Draws the completion popup as a small overlay near the cursor. Prefers the
/// space just below the cursor, flipping above it when there is no room.
fn draw_completion<W: Write>(
    out: &mut W,
    ed: &Editor,
    cur_x: usize,
    cur_y: usize,
    cols: usize,
    rows: usize,
) -> io::Result<()> {
    let Some(c) = &ed.completion else {
        return Ok(());
    };
    if c.items.is_empty() {
        return Ok(());
    }

    let height = c.items.len();
    let max_label = c.items.iter().map(|s| s.chars().count()).max().unwrap_or(0);
    let box_w = (max_label + 2).clamp(1, cols);
    // Keep the box on screen horizontally.
    let x = if cur_x + box_w <= cols {
        cur_x
    } else {
        cols.saturating_sub(box_w)
    };
    // Below the cursor if it fits, otherwise above it.
    let y0 = if cur_y + 1 + height <= rows {
        cur_y + 1
    } else if cur_y >= height {
        cur_y - height
    } else {
        rows.saturating_sub(height)
    };

    let bg = rgb(ed.config.bar_bg);
    let fg = rgb(ed.config.bar_fg);
    let sel_bg = rgb(ed.config.active_tab_bg);
    let sel_fg = rgb(ed.config.active_tab_fg);

    for (i, item) in c.items.iter().enumerate() {
        let y = y0 + i;
        if y >= rows {
            break;
        }
        let label = pad(&format!(" {}", sanitize(item)), box_w);
        let (b, f) = if i == c.selected {
            (sel_bg, sel_fg)
        } else {
            (bg, fg)
        };
        queue!(
            out,
            MoveTo(x as u16, y as u16),
            SetBackgroundColor(b),
            SetForegroundColor(f),
            Print(label),
            ResetColor
        )?;
    }
    Ok(())
}

/// Whether the character at (row, col) is inside the selection.
fn in_selection(
    sel: Option<((usize, usize), (usize, usize))>,
    row: usize,
    col: usize,
) -> bool {
    let Some(((sr, sc), (er, ec))) = sel else {
        return false;
    };
    if row < sr || row > er {
        return false;
    }
    if sr == er {
        col >= sc && col < ec
    } else if row == sr {
        col >= sc
    } else if row == er {
        col < ec
    } else {
        true
    }
}

fn draw_bottom_bar<W: Write>(
    out: &mut W,
    ed: &Editor,
    cols: usize,
    rows: usize,
    bottom: usize,
) -> io::Result<()> {
    let y = (rows - 1) as u16;
    let bar_bg = rgb(ed.config.bar_bg);
    let bar_fg = rgb(ed.config.bar_fg);
    let active_bg = rgb(ed.config.active_tab_bg);
    let active_fg = rgb(ed.config.active_tab_fg);

    if let Mode::NewFile = ed.mode {
        queue!(out, MoveTo(0, y), Clear(ClearType::CurrentLine))?;
        let line = format!("  New file: {}_", sanitize(&ed.prompt));
        queue!(
            out,
            SetBackgroundColor(active_bg),
            SetForegroundColor(active_fg),
            Print(pad(&line, cols)),
            ResetColor
        )?;
        return Ok(());
    }

    // A status message (e.g. a syntax-check result) takes over the bottom rows.
    // The error/headline goes on top; the linter's suggestion, when present,
    // gets its own amber row directly beneath it.
    if !ed.status.is_empty() {
        let ok = ed.status == "Syntax OK";
        let (bg, fg) = if ok {
            (Color::Rgb { r: 34, g: 84, b: 48 }, Color::Rgb { r: 220, g: 255, b: 220 })
        } else {
            (Color::Rgb { r: 120, g: 40, b: 40 }, Color::Rgb { r: 255, g: 224, b: 224 })
        };

        let status_y = (rows - bottom) as u16;
        queue!(out, MoveTo(0, status_y), Clear(ClearType::CurrentLine))?;
        let line = format!("  {}", sanitize(&ed.status));
        queue!(
            out,
            SetBackgroundColor(bg),
            SetForegroundColor(fg),
            Print(pad(&line, cols)),
            ResetColor
        )?;

        if bottom > 1 && !ed.suggestion.is_empty() {
            // Amber "did you mean" row.
            let sg_bg = Color::Rgb { r: 90, g: 70, b: 20 };
            let sg_fg = Color::Rgb { r: 255, g: 226, b: 150 };
            queue!(out, MoveTo(0, y), Clear(ClearType::CurrentLine))?;
            let line = format!("  ↳ {}", sanitize(&ed.suggestion));
            queue!(
                out,
                SetBackgroundColor(sg_bg),
                SetForegroundColor(sg_fg),
                Print(pad(&line, cols)),
                ResetColor
            )?;
        }
        return Ok(());
    }

    queue!(out, MoveTo(0, y), Clear(ClearType::CurrentLine))?;

    // Fill the whole row's background.
    queue!(
        out,
        SetBackgroundColor(bar_bg),
        SetForegroundColor(bar_fg),
        Print(pad("", cols))
    )?;
    queue!(out, MoveTo(0, y))?;

    let mut x = 0usize;
    for (i, b) in ed.buffers.iter().enumerate() {
        let mark = if b.dirty { "●" } else { "" };
        let label = format!(" {}{} ", sanitize(&b.name), mark);
        let w = label.chars().count();
        if x + w > cols {
            break;
        }
        if i == ed.active {
            queue!(
                out,
                SetBackgroundColor(active_bg),
                SetForegroundColor(active_fg),
                Print(&label)
            )?;
        } else {
            queue!(
                out,
                SetBackgroundColor(bar_bg),
                SetForegroundColor(bar_fg),
                Print(&label)
            )?;
        }
        // Separator between tabs.
        queue!(out, SetBackgroundColor(bar_bg), SetForegroundColor(GUTTER_FG))?;
        if i + 1 < ed.buffers.len() && x + w + 1 <= cols {
            queue!(out, Print("│"))?;
        }
        x += w + 1;
    }
    queue!(out, ResetColor)?;
    Ok(())
}
