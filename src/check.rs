use std::fs;
use std::path::Path;
use std::process::Command;

/// Built-in "check only" commands per file extension. The file to check is
/// appended as the last argument. These invoke each language's own tool, so a
/// language is only checkable if its tool is installed on the system.
const CHECKERS: &[(&str, &[&str])] = &[
    ("py", &["python3", "-m", "py_compile"]),
    ("js", &["node", "--check"]),
    ("mjs", &["node", "--check"]),
    ("cjs", &["node", "--check"]),
    ("rb", &["ruby", "-c"]),
    ("php", &["php", "-l"]),
    ("pl", &["perl", "-c"]),
    ("sh", &["bash", "-n"]),
    ("bash", &["bash", "-n"]),
    ("zsh", &["zsh", "-n"]),
    ("lua", &["luac", "-p"]),
    ("go", &["gofmt", "-e"]),
    ("c", &["gcc", "-fsyntax-only"]),
    ("h", &["gcc", "-fsyntax-only"]),
    ("cpp", &["g++", "-fsyntax-only"]),
    ("cc", &["g++", "-fsyntax-only"]),
    ("hpp", &["g++", "-fsyntax-only"]),
    (
        "rs",
        &[
            "rustc",
            "--edition=2021",
            "--crate-type=lib",
            "--emit=metadata",
            "-o",
            "/dev/null",
        ],
    ),
    ("json", &["python3", "-m", "json.tool"]),
    (
        "yaml",
        &["python3", "-c", "import sys,yaml; yaml.safe_load(open(sys.argv[1]))"],
    ),
    (
        "yml",
        &["python3", "-c", "import sys,yaml; yaml.safe_load(open(sys.argv[1]))"],
    ),
];

/// Outcome of a syntax check.
///
/// `message` is a short headline for the status bar. `detail` is the linter's
/// own description of what is wrong (verbatim, never fabricated). `suggestion`
/// is the linter's own "did you mean / help / note" line when it offers one —
/// i.e. what the author probably meant. `error_line`/`error_col` are 0-based and
/// drive the red highlight in the text area.
pub struct CheckResult {
    pub message: String,
    pub detail: Option<String>,
    pub suggestion: Option<String>,
    pub error_line: Option<usize>,
    pub error_col: Option<usize>,
}

impl CheckResult {
    fn ok() -> Self {
        CheckResult {
            message: "Syntax OK".to_string(),
            detail: None,
            suggestion: None,
            error_line: None,
            error_col: None,
        }
    }

    fn plain(message: impl Into<String>) -> Self {
        CheckResult {
            message: message.into(),
            detail: None,
            suggestion: None,
            error_line: None,
            error_col: None,
        }
    }
}

/// Returns the built-in checker command for an extension, if any.
pub fn builtin(ext: &str) -> Option<Vec<String>> {
    CHECKERS
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, cmd)| cmd.iter().map(|s| s.to_string()).collect())
}

/// Runs `cmd` against the buffer content. The content is written to a temp file
/// (keeping the real extension so tools that infer the language from it work),
/// the checker runs on that file, then it is removed. The user's code is never
/// executed beyond what these "check only" tools do themselves.
pub fn run(cmd: &[String], ext: &str, content: &str, dir: &Path) -> CheckResult {
    if cmd.is_empty() {
        return CheckResult::plain("Empty checker command");
    }
    let fname = format!(".btix-check.{ext}");
    let tmp = dir.join(&fname);
    if fs::write(&tmp, content).is_err() {
        return CheckResult::plain("Could not write temp file for checking");
    }

    let output = Command::new(&cmd[0]).args(&cmd[1..]).arg(&tmp).output();
    let _ = fs::remove_file(&tmp);

    match output {
        Err(_) => CheckResult::plain(format!("Checker '{}' not installed", cmd[0])),
        Ok(out) if out.status.success() => CheckResult::ok(),
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(&out.stdout));
            parse_error(&text, &fname)
        }
    }
}

/// Turns raw checker output into a structured result. The line/column come from
/// the first recognizable location; the detail and suggestion are pulled
/// verbatim from the tool's own wording so we never invent a fix.
fn parse_error(text: &str, marker: &str) -> CheckResult {
    let (line, col) = extract_line_col(text, marker);
    let detail = extract_detail(text, marker);
    let suggestion = extract_suggestion(text);

    let message = match line {
        Some(n) => format!("Syntax error at line {n}"),
        None => "Syntax error".to_string(),
    };

    CheckResult {
        message,
        detail,
        suggestion,
        error_line: line.map(|n| n.saturating_sub(1)),
        error_col: col.map(|n| n.saturating_sub(1)),
    }
}

/// Pulls a 1-based (line, column) out of compiler/linter output. Tries the
/// common location formats, in order:
///   "<file>:LINE:COL"  (gcc/clang/rust/node-ish)
///   "<file>:LINE"
///   "<file>(LINE,COL)" (tsc/lua-ish)
///   "line LINE"        (python/ruby fallback, no reliable column)
fn extract_line_col(text: &str, marker: &str) -> (Option<usize>, Option<usize>) {
    for (i, _) in text.match_indices(marker) {
        let rest = &text[i + marker.len()..];
        if let Some(stripped) = rest.strip_prefix(':') {
            if let Some((line, after)) = take_number(stripped) {
                // Optional ":COL" right after the line.
                let col = after
                    .strip_prefix(':')
                    .and_then(|s| take_number(s).map(|(c, _)| c));
                return (Some(line), col);
            }
        }
        if let Some(stripped) = rest.strip_prefix('(') {
            if let Some((line, after)) = take_number(stripped) {
                let col = after
                    .strip_prefix(',')
                    .and_then(|s| take_number(s).map(|(c, _)| c));
                return (Some(line), col);
            }
        }
    }
    // Fallback: "line N" anywhere in the output (no column available).
    let lower = text.to_lowercase();
    if let Some(i) = lower.find("line ") {
        if let Some((line, _)) = take_number(&lower[i + "line ".len()..]) {
            return (Some(line), None);
        }
    }
    (None, None)
}

/// Parses a leading run of digits, returning the value and the remaining text.
fn take_number(s: &str) -> Option<(usize, &str)> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let n = digits.parse::<usize>().ok().filter(|n| *n > 0)?;
    Some((n, &s[digits.len()..]))
}

/// Finds the line that actually describes the problem, e.g.
/// "SyntaxError: invalid syntax" or "error: expected `;`". Falls back to the
/// last non-empty line, which for most tools is the human-readable summary.
fn extract_detail(text: &str, marker: &str) -> Option<String> {
    let keys = [
        "syntaxerror",
        "error:",
        "error[",
        "fatal error",
        "parse error",
        "unexpected",
        "expected",
    ];
    let mut fallback: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        // The descriptive line may share the location prefix (rustc/gcc style),
        // so we don't skip marker lines here — clean_detail strips the prefix.
        if keys.iter().any(|k| lower.contains(k)) {
            return Some(clean_detail(line, marker));
        }
        // For the fallback, avoid lines that just echo the temp file name.
        if !line.contains(marker) {
            fallback = Some(line.to_string());
        }
    }
    fallback.map(|l| clean_detail(&l, marker))
}

/// Strips the temp file name and a leading "<file>:N:N:" location prefix so the
/// shown detail reads as a plain message rather than leaking ".btix-check.*".
fn clean_detail(line: &str, marker: &str) -> String {
    let mut s = line.to_string();
    // Drop a leading location prefix up to the first ": " after the marker.
    if let Some(i) = s.find(marker) {
        let after = &s[i + marker.len()..];
        if let Some(j) = after.find(": ") {
            s = after[j + 2..].to_string();
        }
    }
    s.replace(marker, "this file").trim().to_string()
}

/// Pulls the tool's own "what you probably meant" line, if it offers one.
/// These are the conventional hint markers used by rustc, gcc/clang, perl, etc.
fn extract_suggestion(text: &str) -> Option<String> {
    let keys = ["help:", "note:", "hint:", "did you mean", "perhaps you"];
    for raw in text.lines() {
        let line = raw.trim();
        let lower = line.to_lowercase();
        if let Some(k) = keys.iter().find(|k| lower.contains(*k)) {
            // Keep the text from the marker onward so it reads naturally.
            if let Some(pos) = lower.find(*k) {
                let s = line[pos..].trim();
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_file_colon_line() {
        let out = ".btix-check.js:12\nSyntaxError: bad";
        assert_eq!(extract_line_col(out, ".btix-check.js"), (Some(12), None));
    }

    #[test]
    fn extracts_line_and_column() {
        let out = ".btix-check.rs:3:9: error: expected `;`";
        assert_eq!(extract_line_col(out, ".btix-check.rs"), (Some(3), Some(9)));
    }

    #[test]
    fn extracts_paren_line_col() {
        let out = ".btix-check.ts(7,5): error TS1005";
        assert_eq!(extract_line_col(out, ".btix-check.ts"), (Some(7), Some(5)));
    }

    #[test]
    fn extracts_word_line() {
        let out = "  File \".btix-check.py\", line 4\nSyntaxError";
        // No "<marker>:NN" here, so it falls back to "line N".
        assert_eq!(extract_line_col(out, ".btix-check.py"), (Some(4), None));
    }

    #[test]
    fn no_line_returns_none() {
        assert_eq!(extract_line_col("something went wrong", "x"), (None, None));
    }

    #[test]
    fn detail_prefers_error_line() {
        let out = "  File \".btix-check.py\", line 1\n    x ===\n        ^\nSyntaxError: invalid syntax";
        assert_eq!(
            extract_detail(out, ".btix-check.py").as_deref(),
            Some("SyntaxError: invalid syntax")
        );
    }

    #[test]
    fn detail_strips_location_prefix() {
        let out = ".btix-check.rs:3:9: error: expected `;`, found `let`";
        assert_eq!(
            extract_detail(out, ".btix-check.rs").as_deref(),
            Some("error: expected `;`, found `let`")
        );
    }

    #[test]
    fn suggestion_from_help_line() {
        let out = "error: expected `;`\n help: add a semicolon here";
        assert_eq!(
            extract_suggestion(out).as_deref(),
            Some("help: add a semicolon here")
        );
    }

    #[test]
    fn suggestion_none_when_absent() {
        assert_eq!(extract_suggestion("error: boom"), None);
    }
}
