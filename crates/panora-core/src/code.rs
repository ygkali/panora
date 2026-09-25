// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Heuristic: does a piece of text look like source code rather than
//! prose (UI-10)? Used only to pick a monospace font for display -- it
//! never touches capture, storage, search or `ContentKind`, so getting it
//! wrong costs a font choice, not a wrong decision anywhere that matters.
//! That is also why this can be a little more eager than `sensitive`'s
//! heuristics, which err toward ordinary text because a false positive
//! there hides something the user wanted to see.

/// Below this many characters there is not enough text to score reliably;
/// a bare word or two stays in the ordinary font.
const MIN_LEN: usize = 8;
/// Longest text the heuristic looks at; a whole document is scored the
/// same as a snippet either way, so there is nothing to gain from scanning
/// megabytes of it.
const MAX_LEN: usize = 64 * 1024;
/// A text scoring at least this many points reads as code.
const THRESHOLD: u32 = 3;

/// Substrings that show up in source code far more often than in prose.
/// Deliberately picked with enough surrounding syntax (a trailing `(`, a
/// trailing space, a leading `#`) that they do not fire on the bare word
/// appearing in a sentence -- "return" the button on a form is not "return
/// " the keyword.
const SNIPPETS: &[&str] = &[
    "function(",
    "function (",
    "def ",
    "class ",
    "struct ",
    "impl ",
    "fn ",
    "return ",
    "import ",
    "#include",
    "using namespace",
    "console.log(",
    "public class",
    "private ",
    "protected ",
    "package ",
    "namespace ",
    "SELECT * FROM",
    "SELECT ",
    "INSERT INTO",
    "func ",
    "let ",
    "const ",
    "var ",
];

/// Multi-character operators and punctuation clusters that are close to
/// exclusive to code across the languages Panora's users are likely to
/// paste from.
const CLUSTERS: &[&str] = &["=>", "->", "::", "&&", "||", "==", "!=", "();", "</", "/>"];

/// Characters counted for the symbol-density signal: common in code
/// (braces, brackets, operators), rare relative to letters in prose. `/`
/// is deliberately excluded -- it is common in ordinary paths and URLs.
const SYMBOL_CHARS: &str = "{}[]()<>;=&|!*%^~";

/// Whether `text` reads as source code. See the module docs for what this
/// is (and is not) used for.
pub fn looks_like_code(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < MIN_LEN || trimmed.len() > MAX_LEN {
        return false;
    }

    // A shebang or a JSON/array-shaped document is code (or code-adjacent
    // data) on its own; nothing else needs to agree.
    if trimmed.starts_with("#!") {
        return true;
    }
    if is_bracket_wrapped(trimmed) {
        return true;
    }

    let mut score = 0u32;

    let lines: Vec<&str> = trimmed.lines().collect();
    let indented = lines
        .iter()
        .filter(|l| l.starts_with(' ') || l.starts_with('\t'))
        .count();
    if lines.len() >= 2 && indented >= 2 {
        score += 2;
    }
    if lines
        .iter()
        .any(|l| l.trim_end().ends_with(';') && !l.trim().is_empty())
    {
        score += 1;
    }

    score += SNIPPETS
        .iter()
        .filter(|snippet| trimmed.contains(*snippet))
        .count()
        .min(3) as u32;

    score += CLUSTERS
        .iter()
        .filter(|cluster| trimmed.contains(*cluster))
        .count()
        .min(3) as u32;

    let symbol_count = trimmed
        .chars()
        .filter(|c| SYMBOL_CHARS.contains(*c))
        .count();
    let letter_count = trimmed.chars().filter(|c| c.is_alphabetic()).count().max(1);
    if symbol_count * 12 >= letter_count {
        score += 2;
    }

    score >= THRESHOLD
}

/// `{...}` or `[...]` spanning the whole (trimmed) text -- a JSON object,
/// array, or similar structured document. Only the outermost brackets are
/// checked; this is a display heuristic, not a parser.
fn is_bracket_wrapped(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    matches!(
        (bytes.first(), bytes.last()),
        (Some(b'{'), Some(b'}')) | (Some(b'['), Some(b']'))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_snippets() {
        assert!(looks_like_code("fn main() {\n    println!(\"hi\");\n}"));
        assert!(looks_like_code("def foo(x):\n    return x + 1"));
        assert!(looks_like_code("function add(a, b) {\n  return a + b;\n}"));
        assert!(looks_like_code("#!/bin/bash\necho \"hi\""));
        assert!(looks_like_code("SELECT * FROM users WHERE id = 1;"));
        assert!(looks_like_code(
            "public class Main {\n  public static void main() {}\n}"
        ));
    }

    #[test]
    fn detects_json_and_array_documents() {
        assert!(looks_like_code(r#"{"key": "value", "num": 5}"#));
        assert!(looks_like_code("[1, 2, 3, 4]"));
        assert!(looks_like_code(
            "{\n  \"name\": \"panora\",\n  \"version\": \"1.3.0\"\n}"
        ));
    }

    #[test]
    fn plain_prose_is_never_code() {
        assert!(!looks_like_code(
            "The quick brown fox jumps over the lazy dog."
        ));
        assert!(!looks_like_code(
            "Merhaba dünya — bu bir düz metin kaydı. Kartlar içeriklerine \
             göre boyutlanır ve dört satırdan sonra kısaltılır."
        ));
        assert!(!looks_like_code(
            "- First item\n- Second item\n- Third item"
        ));
        assert!(!looks_like_code(
            "1. First step\n2. Second step\n3. Third step"
        ));
    }

    #[test]
    fn urls_and_paths_are_not_code() {
        assert!(!looks_like_code(
            "https://gitlab.gnome.org/GNOME/mutter/-/merge_requests"
        ));
        assert!(!looks_like_code(
            "/home/user/documents/quarterly-report.pdf"
        ));
        assert!(!looks_like_code("user@example.com"));
    }

    #[test]
    fn very_short_or_very_long_text_is_never_flagged() {
        assert!(!looks_like_code("{}"));
        assert!(!looks_like_code("fn"));
        assert!(!looks_like_code(""));
        let huge = "x".repeat(MAX_LEN + 1);
        assert!(!looks_like_code(&huge));
    }

    #[test]
    fn a_single_statement_with_a_keyword_and_a_semicolon_is_code() {
        assert!(looks_like_code("const answer = 42;"));
        assert!(looks_like_code("let x = compute();"));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// A pure display heuristic must never panic, on any input --
        /// including a clipboard entry no valid UTF-8 text encoder would
        /// produce, if one somehow reaches here.
        #[test]
        fn never_panics_on_arbitrary_text(text in ".{0,500}") {
            let _ = looks_like_code(&text);
        }
    }
}
