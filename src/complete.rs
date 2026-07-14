//! Word completion: language keywords/builtins plus identifiers already present
//! in the open file. There is no semantic analysis here — only prefix matching.

/// Most candidates we ever offer at once.
const MAX_ITEMS: usize = 8;

/// Per-extension keyword/builtin lists. Same shape as the checker table in
/// check.rs. Extensions absent here still complete against words in the buffer.
const KEYWORDS: &[(&str, &[&str])] = &[
    (
        "py",
        &[
            "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
            "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
            "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
            "return", "try", "while", "with", "yield", "abs", "all", "any", "bool", "dict",
            "enumerate", "filter", "float", "format", "input", "int", "isinstance", "len", "list",
            "map", "max", "min", "open", "print", "range", "repr", "reversed", "round", "set",
            "sorted", "str", "sum", "super", "tuple", "type", "zip",
        ],
    ),
    (
        "js",
        &[
            "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger",
            "default", "delete", "do", "else", "export", "extends", "finally", "for", "function",
            "if", "import", "in", "instanceof", "let", "new", "null", "of", "return", "static",
            "super", "switch", "this", "throw", "try", "typeof", "undefined", "var", "void",
            "while", "yield", "console", "document", "window", "Array", "Boolean", "JSON", "Map",
            "Math", "Number", "Object", "Promise", "Set", "String", "Symbol",
        ],
    ),
    ("mjs", JS_LIKE),
    ("cjs", JS_LIKE),
    (
        "ts",
        &[
            "abstract", "any", "as", "async", "await", "boolean", "break", "case", "catch",
            "class", "const", "continue", "declare", "default", "delete", "do", "else", "enum",
            "export", "extends", "finally", "for", "function", "if", "implements", "import", "in",
            "instanceof", "interface", "keyof", "let", "namespace", "never", "new", "null",
            "number", "object", "of", "private", "protected", "public", "readonly", "return",
            "static", "string", "super", "switch", "this", "throw", "try", "type", "typeof",
            "undefined", "unknown", "var", "void", "while", "yield",
        ],
    ),
    (
        "rs",
        &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while", "Box", "Option", "Result",
            "Some", "None", "Ok", "Err", "Vec", "String", "println", "print", "format", "vec",
            "panic", "todo", "unreachable", "derive", "usize", "isize",
        ],
    ),
    ("c", C_LIKE),
    ("h", C_LIKE),
    ("cpp", CPP_LIKE),
    ("cc", CPP_LIKE),
    ("hpp", CPP_LIKE),
    (
        "go",
        &[
            "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough",
            "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range",
            "return", "select", "struct", "switch", "type", "var", "append", "cap", "close",
            "copy", "delete", "len", "make", "new", "nil", "panic", "print", "println", "recover",
            "string", "error", "true", "false",
        ],
    ),
    (
        "rb",
        &[
            "BEGIN", "END", "alias", "and", "begin", "break", "case", "class", "def", "defined?",
            "do", "else", "elsif", "end", "ensure", "false", "for", "if", "in", "module", "next",
            "nil", "not", "or", "redo", "rescue", "retry", "return", "self", "super", "then",
            "true", "undef", "unless", "until", "when", "while", "yield", "puts", "print", "require",
            "attr_accessor", "lambda", "proc",
        ],
    ),
    (
        "php",
        &[
            "abstract", "and", "array", "as", "break", "callable", "case", "catch", "class",
            "clone", "const", "continue", "declare", "default", "do", "echo", "else", "elseif",
            "empty", "enddeclare", "endfor", "endforeach", "endif", "endswitch", "endwhile",
            "extends", "final", "finally", "for", "foreach", "function", "global", "if",
            "implements", "include", "instanceof", "interface", "isset", "list", "namespace",
            "new", "or", "print", "private", "protected", "public", "require", "return", "static",
            "switch", "throw", "trait", "try", "unset", "use", "var", "while", "yield",
        ],
    ),
    ("sh", SH_LIKE),
    ("bash", SH_LIKE),
    ("zsh", SH_LIKE),
    (
        "lua",
        &[
            "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto",
            "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until",
            "while", "print", "pairs", "ipairs", "require", "tostring", "tonumber", "type",
            "pcall", "table", "string", "math",
        ],
    ),
    (
        "java",
        &[
            "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class",
            "const", "continue", "default", "do", "double", "else", "enum", "extends", "final",
            "finally", "float", "for", "if", "implements", "import", "instanceof", "int",
            "interface", "long", "native", "new", "package", "private", "protected", "public",
            "return", "short", "static", "super", "switch", "synchronized", "this", "throw",
            "throws", "transient", "try", "void", "volatile", "while", "String", "System",
            "println", "print", "true", "false", "null",
        ],
    ),
];

const JS_LIKE: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete",
    "do", "else", "export", "extends", "finally", "for", "function", "if", "import", "in",
    "instanceof", "let", "new", "null", "of", "return", "static", "super", "switch", "this",
    "throw", "try", "typeof", "undefined", "var", "void", "while", "yield", "console", "Array",
    "JSON", "Math", "Object", "Promise",
];

const C_LIKE: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else", "enum",
    "extern", "float", "for", "goto", "if", "int", "long", "register", "return", "short", "signed",
    "sizeof", "static", "struct", "switch", "typedef", "union", "unsigned", "void", "volatile",
    "while", "include", "define", "printf", "scanf", "malloc", "free", "sizeof", "NULL",
];

const CPP_LIKE: &[&str] = &[
    "auto", "bool", "break", "case", "catch", "char", "class", "const", "constexpr", "continue",
    "default", "delete", "do", "double", "else", "enum", "explicit", "export", "extern", "false",
    "float", "for", "friend", "goto", "if", "inline", "int", "long", "namespace", "new", "nullptr",
    "operator", "private", "protected", "public", "return", "short", "signed", "sizeof", "static",
    "struct", "switch", "template", "this", "throw", "true", "try", "typedef", "typename", "union",
    "unsigned", "using", "virtual", "void", "volatile", "while", "include", "printf", "std",
    "cout", "cin", "endl", "string", "vector",
];

const SH_LIKE: &[&str] = &[
    "if", "then", "elif", "else", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "function", "in", "select", "return", "break", "continue", "local", "export", "readonly",
    "echo", "printf", "read", "exit", "test", "source", "alias", "unset", "shift",
];

/// Returns the trailing run of identifier characters immediately before the
/// cursor on `line`. `cx` is a character index. Empty when the cursor is not
/// right after a word.
pub fn word_prefix(line: &str, cx: usize) -> String {
    let chars: Vec<char> = line.chars().collect();
    let end = cx.min(chars.len());
    let mut start = end;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    chars[start..end].iter().collect()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Completion candidates for `prefix`: language keywords for `ext` plus every
/// identifier found in `lines`. Excludes the prefix itself, dedupes, sorts so
/// the shortest (closest) match comes first, and caps the count.
pub fn candidates(ext: &str, lines: &[String], prefix: &str) -> Vec<String> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let push = |w: &str, out: &mut Vec<String>| {
        if w != prefix && w.starts_with(prefix) && !out.iter().any(|e| e == w) {
            out.push(w.to_string());
        }
    };

    if let Some((_, words)) = KEYWORDS.iter().find(|(e, _)| *e == ext) {
        for w in *words {
            push(w, &mut out);
        }
    }
    for line in lines {
        for word in split_identifiers(line) {
            push(word, &mut out);
        }
    }

    out.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    out.truncate(MAX_ITEMS);
    out
}

/// Yields identifier tokens from a line: maximal runs of word characters that
/// do not start with a digit (so numbers are not offered as completions).
fn split_identifiers(line: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < line.len() {
        // Advance to the next char boundary-safe word start.
        let ch = line[i..].chars().next().unwrap();
        let len = ch.len_utf8();
        if is_word_char(ch) {
            let start = i;
            let mut j = i;
            while j < line.len() {
                let c = line[j..].chars().next().unwrap();
                if is_word_char(c) {
                    j += c.len_utf8();
                } else {
                    break;
                }
            }
            let tok = &line[start..j];
            if !tok.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(true) {
                tokens.push(tok);
            }
            i = j;
        } else {
            i += len;
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_prefix_trailing_run() {
        assert_eq!(word_prefix("x = pri", 7), "pri");
        assert_eq!(word_prefix("foo.bar", 7), "bar");
        assert_eq!(word_prefix("foo()", 5), "");
        assert_eq!(word_prefix("hello", 3), "hel");
    }

    #[test]
    fn keyword_candidate_for_prefix() {
        let lines = vec!["pri".to_string()];
        let c = candidates("py", &lines, "pri");
        assert!(c.contains(&"print".to_string()), "got {c:?}");
    }

    #[test]
    fn exact_prefix_excluded() {
        let lines: Vec<String> = vec![];
        let c = candidates("py", &lines, "print");
        assert!(!c.contains(&"print".to_string()));
    }

    #[test]
    fn buffer_identifier_surfaces() {
        let lines = vec!["my_variable = 1".to_string(), "my_".to_string()];
        let c = candidates("txt", &lines, "my_");
        assert!(c.contains(&"my_variable".to_string()), "got {c:?}");
    }

    #[test]
    fn no_match_is_empty() {
        let lines = vec!["abc".to_string()];
        assert!(candidates("py", &lines, "zzzq").is_empty());
    }

    #[test]
    fn numbers_not_offered() {
        let lines = vec!["1234567".to_string()];
        assert!(candidates("txt", &lines, "12").is_empty());
    }

    #[test]
    fn shortest_match_first() {
        let lines = vec!["set setattr setdefault".to_string()];
        let c = candidates("py", &lines, "se");
        assert_eq!(c.first().map(|s| s.as_str()), Some("set"));
    }
}
