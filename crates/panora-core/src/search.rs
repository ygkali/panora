// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The search box grammar, shared by the popup and the CLI.
//!
//! Free words match as prefixes in the full-text index, `"quoted phrases"`
//! match exactly, `key:value` operators narrow the list, and `re:` turns the
//! rest of the line into a regular expression matched against the preview
//! and the indexed text:
//!
//! ```text
//! kind:image app:firefox after:7d before:2026-09-01 pinned:yes "exact phrase" word
//! re:^https?://.*\.pdf$
//! ```
//!
//! Operators: `kind:` (text, richtext, link, image, files, color, binary and
//! a few aliases), `app:` (substring of the source application), `pinned:`
//! (yes/no), `before:` and `after:` (a `YYYY-MM-DD` day or a relative
//! `30m`, `12h`, `7d`, `2w`). A token whose value the grammar does not
//! understand is searched for as a word.

use crate::config::{MAX_PATTERN_LEN, PATTERN_SIZE_LIMIT};
use crate::error::{Error, Result};
use crate::model::ContentKind;

/// What a search string asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedQuery {
    /// Words, each matched as a prefix.
    pub terms: Vec<String>,
    /// Quoted phrases, matched exactly.
    pub phrases: Vec<String>,
    /// `kind:`.
    pub kind: Option<ContentKind>,
    /// `app:`, lowercased.
    pub app: Option<String>,
    /// `pinned:`.
    pub pinned: Option<bool>,
    /// `before:` as a Unix timestamp; entries last seen earlier match.
    pub before: Option<i64>,
    /// `after:` as a Unix timestamp; entries last seen at or after match.
    pub after: Option<i64>,
    /// `re:`, everything after it.
    pub regex: Option<String>,
}

/// How many words and phrases go into one FTS expression.
const MAX_TERMS: usize = 16;
const MAX_PHRASES: usize = 8;

impl ParsedQuery {
    /// The FTS5 `MATCH` expression for the words and phrases, if any.
    ///
    /// `"` is stripped so a term or phrase can never close its quoted span
    /// early. `\0` is stripped too: SQLite's FTS5 query-string parser scans
    /// the `MATCH` argument as if it were NUL-terminated even though it
    /// arrives as length-prefixed TEXT, so an embedded NUL truncates the
    /// scan mid-quote and the whole query fails with "unterminated string"
    /// instead of just not matching that byte.
    pub fn fts_expression(&self) -> Option<String> {
        let mut parts: Vec<String> = self
            .terms
            .iter()
            .take(MAX_TERMS)
            .map(|term| format!("\"{}\"*", term.replace(['"', '\0'], "")))
            .collect();
        parts.extend(
            self.phrases
                .iter()
                .take(MAX_PHRASES)
                .map(|phrase| format!("\"{}\"", phrase.replace(['"', '\0'], ""))),
        );
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        }
    }

    /// True when the string held nothing the grammar could use.
    pub fn is_empty(&self) -> bool {
        self == &ParsedQuery::default()
    }
}

/// Parse a search string. `now` (Unix seconds) anchors the relative times.
pub fn parse(input: &str, now: i64) -> ParsedQuery {
    let mut parsed = ParsedQuery::default();
    let mut rest = input.trim();
    while !rest.is_empty() {
        if let Some(pattern) = rest.strip_prefix("re:") {
            let pattern = pattern.trim();
            if !pattern.is_empty() {
                parsed.regex = Some(pattern.to_string());
            }
            break;
        }
        let (token, remainder) = next_token(rest);
        rest = remainder.trim_start();
        match token {
            Token::Phrase(phrase) => {
                let phrase = phrase.trim();
                if !phrase.is_empty() {
                    parsed.phrases.push(phrase.to_string());
                }
            }
            Token::Word(word) => {
                if let Some((key, value)) = word.split_once(':') {
                    let value = value.trim_matches('"');
                    if apply_operator(&mut parsed, key, value, now) {
                        continue;
                    }
                }
                let word = word.trim_matches('"');
                if !word.is_empty() {
                    parsed.terms.push(word.to_string());
                }
            }
        }
    }
    parsed
}

enum Token<'a> {
    Word(&'a str),
    Phrase(&'a str),
}

/// The next token: a quoted phrase, or a word that may carry a quoted
/// operator value (`app:"Text Editor"`).
fn next_token(s: &str) -> (Token<'_>, &str) {
    if let Some(inner) = s.strip_prefix('"') {
        return match inner.find('"') {
            Some(end) => (Token::Phrase(&inner[..end]), &inner[end + 1..]),
            None => (Token::Phrase(inner), ""),
        };
    }
    let mut in_quotes = false;
    let mut end = s.len();
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    (Token::Word(&s[..end]), &s[end..])
}

fn apply_operator(parsed: &mut ParsedQuery, key: &str, value: &str, now: i64) -> bool {
    match key.to_ascii_lowercase().as_str() {
        "kind" | "type" => match kind_alias(value) {
            Some(kind) => {
                parsed.kind = Some(kind);
                true
            }
            None => false,
        },
        "app" | "source" => {
            if value.is_empty() {
                return false;
            }
            parsed.app = Some(value.to_lowercase());
            true
        }
        "pinned" | "pin" => match value.to_ascii_lowercase().as_str() {
            "yes" | "true" | "1" | "on" => {
                parsed.pinned = Some(true);
                true
            }
            "no" | "false" | "0" | "off" => {
                parsed.pinned = Some(false);
                true
            }
            _ => false,
        },
        "is" => match value.to_ascii_lowercase().as_str() {
            "pinned" => {
                parsed.pinned = Some(true);
                true
            }
            "unpinned" => {
                parsed.pinned = Some(false);
                true
            }
            _ => false,
        },
        "before" => match parse_time(value, now) {
            Some(ts) => {
                parsed.before = Some(ts);
                true
            }
            None => false,
        },
        "after" | "since" => match parse_time(value, now) {
            Some(ts) => {
                parsed.after = Some(ts);
                true
            }
            None => false,
        },
        _ => false,
    }
}

fn kind_alias(value: &str) -> Option<ContentKind> {
    Some(match value.to_ascii_lowercase().as_str() {
        "text" | "txt" | "plain" => ContentKind::Text,
        "richtext" | "rich" | "html" | "rtf" => ContentKind::RichText,
        "link" | "links" | "url" | "urls" => ContentKind::Link,
        "image" | "images" | "img" | "picture" | "pictures" => ContentKind::Image,
        "file" | "files" => ContentKind::FileList,
        "color" | "colour" | "colors" | "colours" => ContentKind::Color,
        "binary" | "bin" => ContentKind::Binary,
        _ => return None,
    })
}

/// `7d`-style relative durations, or a `YYYY-MM-DD` day (UTC midnight).
fn parse_time(value: &str, now: i64) -> Option<i64> {
    if let Some((digits, unit)) = value
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| value.split_at(i))
    {
        if !digits.is_empty() {
            let amount: i64 = digits.parse().ok()?;
            let seconds = match unit {
                "s" | "sec" => 1,
                "m" | "min" => 60,
                "h" | "hour" | "hours" => 3600,
                "d" | "day" | "days" => 86_400,
                "w" | "week" | "weeks" => 7 * 86_400,
                _ => return parse_day(value),
            };
            return Some(now.saturating_sub(amount.saturating_mul(seconds)));
        }
    }
    parse_day(value)
}

fn parse_day(value: &str) -> Option<i64> {
    let mut parts = value.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (i64::from(month) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Compile a user-written regular expression with the same limits as the
/// ignore patterns; matching is case-insensitive.
pub fn compile_regex(pattern: &str) -> Result<regex::Regex> {
    if pattern.len() > MAX_PATTERN_LEN {
        return Err(Error::Config("regular expression too long".into()));
    }
    regex::RegexBuilder::new(pattern)
        .size_limit(PATTERN_SIZE_LIMIT)
        .case_insensitive(true)
        .build()
        .map_err(|e| Error::Config(format!("regular expression: {e}")))
}

/// Pango markup for `text` with every match of the query in bold. The text
/// is escaped, so the result is safe for `Label::set_markup` whatever the
/// clipboard held.
pub fn mark_matches(text: &str, query: &ParsedQuery) -> String {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    if let Some(re) = query.regex.as_deref().and_then(|p| compile_regex(p).ok()) {
        spans.extend(re.find_iter(text).map(|m| (m.start(), m.end())));
    }
    let folded = Folded::new(text);
    for needle in query.terms.iter().chain(&query.phrases) {
        let needle = fold_string(needle);
        if needle.is_empty() {
            continue;
        }
        let mut from = 0;
        while let Some(at) = folded.text[from..].find(&needle) {
            let start = from + at;
            let end = start + needle.len();
            spans.push((folded.starts[start], folded.ends[end - 1]));
            from = end;
        }
    }
    if spans.is_empty() {
        return escape_markup(text);
    }
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let mut out = String::with_capacity(text.len() + merged.len() * 7);
    let mut cursor = 0;
    for (start, end) in merged {
        out.push_str(&escape_markup(&text[cursor..start]));
        out.push_str("<b>");
        out.push_str(&escape_markup(&text[start..end]));
        out.push_str("</b>");
        cursor = end;
    }
    out.push_str(&escape_markup(&text[cursor..]));
    out
}

/// `text` lowercased and stripped of common diacritics, with the original
/// byte range of every folded byte, so a match maps back to the original.
struct Folded {
    text: String,
    starts: Vec<usize>,
    ends: Vec<usize>,
}

impl Folded {
    fn new(text: &str) -> Self {
        let mut folded = Folded {
            text: String::with_capacity(text.len()),
            starts: Vec::with_capacity(text.len()),
            ends: Vec::with_capacity(text.len()),
        };
        for (offset, c) in text.char_indices() {
            let end = offset + c.len_utf8();
            let before = folded.text.len();
            push_folded(&mut folded.text, c);
            for _ in before..folded.text.len() {
                folded.starts.push(offset);
                folded.ends.push(end);
            }
        }
        folded
    }
}

fn fold_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        push_folded(&mut out, c);
    }
    out
}

/// One character, lowercased and with the diacritics of the Latin scripts
/// the interface targets removed (what FTS5's `remove_diacritics` does).
fn push_folded(out: &mut String, c: char) {
    let mapped = match c {
        'À'..='Å' | 'à'..='å' => 'a',
        'Ç' | 'ç' => 'c',
        'È'..='Ë' | 'è'..='ë' => 'e',
        'Ì'..='Ï' | 'ì'..='ï' | 'İ' | 'ı' => 'i',
        'Ñ' | 'ñ' => 'n',
        'Ò'..='Ö' | 'ò'..='ö' | 'Ø' | 'ø' => 'o',
        'Ù'..='Ü' | 'ù'..='ü' => 'u',
        'Ý' | 'ý' | 'ÿ' => 'y',
        'Ş' | 'ş' => 's',
        'Ğ' | 'ğ' => 'g',
        'ß' => 's',
        '\u{300}'..='\u{36f}' => return,
        other => {
            for lower in other.to_lowercase() {
                out.push(lower);
            }
            return;
        }
    };
    out.push(mapped);
}

/// Escape for Pango markup.
pub fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn words_and_phrases_become_an_fts_expression() {
        let q = parse(r#"merhaba "tam ifade" dünya"#, NOW);
        assert_eq!(q.terms, vec!["merhaba", "dünya"]);
        assert_eq!(q.phrases, vec!["tam ifade"]);
        assert_eq!(
            q.fts_expression().unwrap(),
            r#""merhaba"* "dünya"* "tam ifade""#
        );
        assert!(parse("   ", NOW).is_empty());
        assert!(parse("", NOW).fts_expression().is_none());
    }

    #[test]
    fn embedded_nul_is_stripped_from_the_fts_expression() {
        // SQLite's FTS5 query-string parser scans MATCH text as if
        // NUL-terminated; an unstripped '\0' truncates the scan mid-quote
        // and the query fails outright ("unterminated string") instead of
        // just not matching that byte. Caught by QA-07's fts_query proptest.
        let q = parse("a\0b \"c\0d\"", NOW);
        assert_eq!(q.fts_expression().unwrap(), r#""ab"* "cd""#);
    }

    #[test]
    fn operators_narrow_the_query() {
        let q = parse(
            r#"kind:image app:"Text Editor" pinned:yes after:7d before:2026-09-01 invoice"#,
            NOW,
        );
        assert_eq!(q.kind, Some(ContentKind::Image));
        assert_eq!(q.app.as_deref(), Some("text editor"));
        assert_eq!(q.pinned, Some(true));
        assert_eq!(q.after, Some(NOW - 7 * 86_400));
        assert_eq!(q.before, Some(days_from_civil(2026, 9, 1) * 86_400));
        assert_eq!(q.terms, vec!["invoice"]);
        assert_eq!(parse("is:unpinned", NOW).pinned, Some(false));
        assert_eq!(parse("type:url", NOW).kind, Some(ContentKind::Link));
        assert_eq!(parse("since:12h", NOW).after, Some(NOW - 12 * 3600));
    }

    #[test]
    fn unknown_operator_values_are_ordinary_words() {
        let q = parse("kind:whatever before:soon http://x:1", NOW);
        assert!(q.kind.is_none() && q.before.is_none());
        assert_eq!(q.terms, vec!["kind:whatever", "before:soon", "http://x:1"]);
    }

    #[test]
    fn re_takes_the_rest_of_the_line() {
        let q = parse(r"kind:link re:^https?://.*\.pdf$ not a word", NOW);
        assert_eq!(q.kind, Some(ContentKind::Link));
        assert_eq!(q.regex.as_deref(), Some(r"^https?://.*\.pdf$ not a word"));
        assert!(q.terms.is_empty());
        assert!(parse("re:", NOW).regex.is_none());
        assert!(compile_regex("(").is_err());
        assert!(compile_regex("^a+$").is_ok());
    }

    #[test]
    fn days_are_counted_from_the_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2026, 9, 20), 20_716);
        assert!(parse_day("2026-13-01").is_none());
        assert!(parse_day("2026-09").is_none());
    }

    #[test]
    fn matches_are_bold_and_the_text_is_escaped() {
        let q = parse("istan <b>", NOW);
        let marked = mark_matches("İstanbul & <b>bold</b>", &q);
        assert_eq!(
            marked,
            "<b>İstan</b>bul &amp; <b>&lt;b&gt;</b>bold&lt;/b&gt;"
        );
        assert_eq!(mark_matches("no hit", &parse("zzz", NOW)), "no hit");
        assert_eq!(mark_matches("a & b", &ParsedQuery::default()), "a &amp; b");
    }

    #[test]
    fn overlapping_and_regex_matches_merge() {
        let q = parse("ab bc", NOW);
        assert_eq!(mark_matches("xabcx", &q), "x<b>abc</b>x");
        let q = parse(r"re:\d+", NOW);
        assert_eq!(
            mark_matches("v1.30 and 7", &q),
            "v<b>1</b>.<b>30</b> and <b>7</b>"
        );
    }

    #[test]
    fn folding_matches_the_index() {
        assert_eq!(fold_string("ŞİŞLİ Çağlayan"), "sisli caglayan");
        assert_eq!(fold_string("Ünye"), "unye");
    }
}
