use crossterm::style::Color;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, ThemeSet};
use syntect::parsing::SyntaxSet;

/// Wrapper over syntect: holds the loaded syntaxes and themes.
pub struct Highlighter {
    ps: SyntaxSet,
    ts: ThemeSet,
    theme: String,
}

impl Highlighter {
    pub fn new(theme: String) -> Self {
        let ts = ThemeSet::load_defaults();
        // Fall back to a built-in theme if the configured one is missing.
        let theme = if ts.themes.contains_key(&theme) {
            theme
        } else {
            "base16-ocean.dark".to_string()
        };
        Highlighter {
            ps: SyntaxSet::load_defaults_newlines(),
            ts,
            theme,
        }
    }

    /// Highlights buffer lines from 0 to `end` (inclusive) but returns results
    /// only for the [start, end] range. We start from the top of the file so
    /// multi-line constructs (strings, comments) are handled correctly.
    pub fn highlight(
        &self,
        lines: &[String],
        ext: &str,
        start: usize,
        end: usize,
    ) -> Vec<Vec<(Color, String)>> {
        let syntax = self
            .ps
            .find_syntax_by_extension(ext)
            .unwrap_or_else(|| self.ps.find_syntax_plain_text());
        let theme = &self.ts.themes[&self.theme];
        let mut hl = HighlightLines::new(syntax, theme);

        let mut out = Vec::new();
        let last = end.min(lines.len().saturating_sub(1));
        for (i, line) in lines.iter().enumerate() {
            if i > last {
                break;
            }
            let with_nl = format!("{}\n", line);
            let ranges = hl.highlight_line(&with_nl, &self.ps).unwrap_or_default();
            if i >= start {
                let spans = ranges
                    .into_iter()
                    .map(|(style, text)| {
                        (to_color(style), text.trim_end_matches('\n').to_string())
                    })
                    .collect();
                out.push(spans);
            }
        }
        out
    }
}

fn to_color(style: Style) -> Color {
    let fg = style.foreground;
    Color::Rgb {
        r: fg.r,
        g: fg.g,
        b: fg.b,
    }
}
