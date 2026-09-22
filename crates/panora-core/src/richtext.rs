// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! HTML to Pango markup, for a limited rich-text preview (UI-22).
//!
//! Clipboard HTML comes from whatever the source application wrote --
//! browsers, editors, chat clients -- so it is treated as untrusted,
//! possibly malformed markup, not a document to render faithfully. This
//! converts a small allowlist of tags (bold, italic, underline,
//! strikethrough, inline code, links, paragraph and line breaks) to their
//! Pango markup equivalents, drops everything else (keeping its text), and
//! discards `<script>`/`<style>` content outright. The output is always
//! well-formed Pango markup: any tag left open by malformed or truncated
//! input is closed at the end rather than handed to Pango unbalanced.
//!
//! This is a preview, not a renderer: no RTF, no images, no tables, no
//! colours or fonts from `style=`.

use crate::search::escape_markup;

/// Longest input scanned; a whole document renders the same as a snippet
/// either way, so there is no reason to walk megabytes of markup for a
/// clipboard preview.
const MAX_LEN: usize = 256 * 1024;

/// What an HTML tag becomes in the Pango markup output.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TagAction {
    /// Not on the allowlist: drop the tag, keep its content flowing.
    Ignore,
    /// A single line break, emitted immediately; nothing to close later.
    Break,
    /// A paragraph-level element: a break before its content, nothing to
    /// close later (its own closing tag emits another break).
    Block,
    /// Maps to a real Pango inline tag; pushed on the open-tag stack so it
    /// can be force-closed if the input never balances it.
    Inline(&'static str),
    /// A paragraph-level element whose content is also bold (headings).
    BlockInline(&'static str),
    /// `<a href="...">`: like `Inline("a")`, but the opening tag also
    /// needs the (escaped) href pulled out of the source attributes.
    Anchor,
    /// `<script>`/`<style>`: their content is not text to show at all.
    SkipContent,
}

fn classify(name: &str) -> TagAction {
    match name {
        "b" | "strong" => TagAction::Inline("b"),
        "i" | "em" => TagAction::Inline("i"),
        "u" | "ins" => TagAction::Inline("u"),
        "s" | "strike" | "del" => TagAction::Inline("s"),
        "code" | "tt" | "kbd" | "samp" | "pre" => TagAction::Inline("tt"),
        "a" => TagAction::Anchor,
        "br" => TagAction::Break,
        "p" | "div" | "li" | "tr" | "blockquote" => TagAction::Block,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => TagAction::BlockInline("b"),
        "script" | "style" => TagAction::SkipContent,
        _ => TagAction::Ignore,
    }
}

/// Convert a clipboard HTML payload to Pango markup suitable for
/// `gtk::TextBuffer::insert_markup` or `gtk::Label::set_markup`.
pub fn html_to_pango(html: &str) -> String {
    let html = truncate_to_char_boundary(html, MAX_LEN);

    let mut out = String::with_capacity(html.len());
    let mut open: Vec<&'static str> = Vec::new();
    let mut skipping: Option<&'static str> = None;
    let mut text_run = String::new();

    let mut chars = html.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '<' {
            if skipping.is_none() {
                push_decoded(&mut text_run, c, &mut chars);
            }
            continue;
        }
        // A `<` with no closing `>` before the end of input is not a tag;
        // treat the rest of the input as ordinary (if unterminated) text.
        let Some(end) = find_tag_end(html, i) else {
            if skipping.is_none() {
                text_run.push('<');
            }
            break;
        };
        let raw = &html[i + 1..end];
        while chars.peek().is_some_and(|&(j, _)| j < end) {
            chars.next();
        }
        chars.next(); // consume the closing `>`

        if !text_run.is_empty() {
            out.push_str(&escape_markup(&text_run));
            text_run.clear();
        }

        let (closing, body) = match raw.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, raw),
        };
        let body = body.strip_suffix('/').unwrap_or(body).trim();
        let name = body
            .split(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();

        if let Some(tag) = skipping {
            if closing && name == tag {
                skipping = None;
            }
            continue;
        }

        apply_tag(&name, body, closing, &mut out, &mut open, &mut skipping);
    }
    if !text_run.is_empty() {
        out.push_str(&escape_markup(&text_run));
    }
    for tag in open.iter().rev() {
        out.push_str(&format!("</{tag}>"));
    }

    collapse_blank_runs(out.trim())
}

fn apply_tag(
    name: &str,
    body: &str,
    closing: bool,
    out: &mut String,
    open: &mut Vec<&'static str>,
    skipping: &mut Option<&'static str>,
) {
    match (classify(name), closing) {
        (TagAction::Ignore, _) => {}
        (TagAction::Break, _) => out.push('\n'),
        (TagAction::Block, _) => out.push_str("\n\n"),
        (TagAction::Inline(tag), false) => {
            open.push(tag);
            out.push('<');
            out.push_str(tag);
            out.push('>');
        }
        (TagAction::Inline(tag), true) => close_if_open(tag, out, open),
        (TagAction::BlockInline(tag), false) => {
            out.push_str("\n\n");
            open.push(tag);
            out.push('<');
            out.push_str(tag);
            out.push('>');
        }
        (TagAction::BlockInline(tag), true) => {
            close_if_open(tag, out, open);
            out.push_str("\n\n");
        }
        (TagAction::Anchor, false) => {
            if let Some(href) = find_attr(body, "href") {
                open.push("a");
                out.push_str("<a href=\"");
                out.push_str(&escape_markup(&href));
                out.push_str("\">");
            }
            // No usable href: treat like `Ignore` (drop the tag, keep the
            // link text flowing as plain text).
        }
        (TagAction::Anchor, true) => close_if_open("a", out, open),
        (TagAction::SkipContent, false) => {
            let tag = if name == "style" { "style" } else { "script" };
            *skipping = Some(tag);
        }
        (TagAction::SkipContent, true) => {}
    }
}

/// Close `tag` only if it is the innermost open tag, matching how the tags
/// this module emits are always properly nested by construction; a
/// mismatched or stray closing tag from malformed input is ignored rather
/// than corrupting the stack (or the output).
fn close_if_open(tag: &'static str, out: &mut String, open: &mut Vec<&'static str>) {
    if open.last() == Some(&tag) {
        open.pop();
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
    }
}

/// The index of the `>` that closes the tag starting at `open_lt` (the
/// index of its `<`), or `None` if the input ends first.
fn find_tag_end(html: &str, open_lt: usize) -> Option<usize> {
    html[open_lt..].find('>').map(|offset| open_lt + offset)
}

/// Consume and decode one HTML entity starting at `&`, or push the
/// character as-is if it does not start a recognized entity.
fn push_decoded(out: &mut String, c: char, chars: &mut std::iter::Peekable<std::str::CharIndices>) {
    if c != '&' {
        out.push(c);
        return;
    }
    let mut entity = String::new();
    let mut consumed = Vec::new();
    while let Some(&(_, next)) = chars.peek() {
        if next == ';' || entity.len() > 12 {
            break;
        }
        entity.push(next);
        consumed.push(next);
        chars.next();
    }
    if chars.peek().map(|&(_, c)| c) == Some(';') {
        chars.next();
        if let Some(decoded) = decode_entity(&entity) {
            out.push(decoded);
            return;
        }
        // Unrecognized `&name;`: show it literally rather than eating it.
        out.push('&');
        out.push_str(&entity);
        out.push(';');
        return;
    }
    // No terminating `;` within the lookahead window: not an entity: put
    // back what was peeked and emit the `&` on its own.
    out.push('&');
    out.push_str(&entity);
}

fn decode_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "mdash" => '—',
        "ndash" => '–',
        "hellip" => '…',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "ldquo" => '\u{201C}',
        "rdquo" => '\u{201D}',
        _ => {
            let code = name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()));
            return code.and_then(char::from_u32);
        }
    })
}

/// The first `href="..."` or `href='...'` attribute value in a tag's raw
/// attribute text, entity-decoded. `None` when there is no href to find,
/// so callers never emit a Pango `<a>` tag without one (Pango rejects it).
fn find_attr(body: &str, attr: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let key = format!("{attr}=");
    let start = lower.find(&key)? + key.len();
    let quote = body.as_bytes().get(start).copied()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let value_start = start + 1;
    let value_end = body[value_start..].find(quote as char)? + value_start;
    let raw = &body[value_start..value_end];
    let mut decoded = String::with_capacity(raw.len());
    let mut chars = raw.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        push_decoded(&mut decoded, c, &mut chars);
    }
    Some(decoded)
}

/// Three or more consecutive newlines (from adjacent block elements)
/// collapse to exactly two, so nested `<div><p>...` does not pile up blank
/// lines no HTML author intended to be visible.
fn collapse_blank_runs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newlines = 0;
    for c in text.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(c);
            }
        } else {
            newlines = 0;
            out.push(c);
        }
    }
    out
}

fn truncate_to_char_boundary(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_formatting_maps_to_pango_tags() {
        assert_eq!(html_to_pango("<b>bold</b>"), "<b>bold</b>");
        assert_eq!(html_to_pango("<strong>bold</strong>"), "<b>bold</b>");
        assert_eq!(html_to_pango("<i>italic</i>"), "<i>italic</i>");
        assert_eq!(html_to_pango("<em>italic</em>"), "<i>italic</i>");
        assert_eq!(html_to_pango("<u>under</u>"), "<u>under</u>");
        assert_eq!(html_to_pango("<s>gone</s>"), "<s>gone</s>");
        assert_eq!(
            html_to_pango("<code>let x = 1;</code>"),
            "<tt>let x = 1;</tt>"
        );
    }

    #[test]
    fn nested_formatting_is_preserved() {
        assert_eq!(
            html_to_pango("<b>bold and <i>also italic</i></b>"),
            "<b>bold and <i>also italic</i></b>"
        );
    }

    #[test]
    fn links_keep_their_href() {
        assert_eq!(
            html_to_pango(r#"<a href="https://example.com">click</a>"#),
            "<a href=\"https://example.com\">click</a>"
        );
        // Single-quoted attribute.
        assert_eq!(
            html_to_pango("<a href='https://example.com'>click</a>"),
            "<a href=\"https://example.com\">click</a>"
        );
    }

    #[test]
    fn a_link_with_no_href_drops_the_tag_but_keeps_the_text() {
        assert_eq!(html_to_pango("<a>click</a>"), "click");
    }

    #[test]
    fn unrecognized_tags_are_stripped_but_their_text_kept() {
        assert_eq!(
            html_to_pango(r#"<span style="color:red">red text</span>"#),
            "red text"
        );
        assert_eq!(html_to_pango("<font face=\"Arial\">hi</font>"), "hi");
    }

    #[test]
    fn script_and_style_content_is_dropped_entirely() {
        assert_eq!(
            html_to_pango("before<script>alert('x')</script>after"),
            "beforeafter"
        );
        assert_eq!(html_to_pango("<style>.a{color:red}</style>text"), "text");
    }

    #[test]
    fn br_and_p_become_line_and_paragraph_breaks() {
        assert_eq!(html_to_pango("line one<br>line two"), "line one\nline two");
        assert_eq!(
            html_to_pango("<p>first</p><p>second</p>"),
            "first\n\nsecond"
        );
    }

    #[test]
    fn headings_are_bold_and_break_the_paragraph() {
        assert_eq!(
            html_to_pango("<h1>Title</h1><p>Body text</p>"),
            "<b>Title</b>\n\nBody text"
        );
    }

    #[test]
    fn entities_are_decoded_then_markup_escaped() {
        assert_eq!(html_to_pango("Ben &amp; Jerry"), "Ben &amp; Jerry");
        assert_eq!(html_to_pango("5 &lt; 10"), "5 &lt; 10");
        assert_eq!(html_to_pango("&copy; 2026"), "© 2026");
        // An entity not in the curated list is shown literally, not eaten.
        assert_eq!(html_to_pango("AB&qux;CD"), "AB&amp;qux;CD");
    }

    #[test]
    fn literal_markup_characters_in_text_are_escaped() {
        // A `<` typed as ordinary text, not a real tag (no matching `>`
        // that looks like one), must not corrupt the Pango markup.
        assert_eq!(
            html_to_pango("5 &lt; 10 &amp; 20 &gt; 3"),
            "5 &lt; 10 &amp; 20 &gt; 3"
        );
        // `escape_markup` also entity-escapes quotes, which is valid (if
        // unnecessary outside an attribute) markup -- Pango renders
        // `&quot;` back to a literal `"` either way.
        assert_eq!(
            html_to_pango("a \"quoted\" word"),
            "a &quot;quoted&quot; word"
        );
    }

    #[test]
    fn unbalanced_or_malformed_markup_never_produces_broken_output() {
        // An opening tag with no closing tag at all: force-closed at EOF.
        assert_eq!(html_to_pango("<b>never closed"), "<b>never closed</b>");
        // A stray closing tag with nothing open: ignored, not underflowed.
        assert_eq!(html_to_pango("stray</b> close"), "stray close");
        // Improperly nested tags still yield valid, fully-closed markup.
        let out = html_to_pango("<b><i>bi</b>i-only</i>");
        assert!(out.starts_with("<b><i>bi"));
        assert!(!out.contains("<b><i><b>"));
    }

    #[test]
    fn plain_text_with_no_tags_round_trips_escaped() {
        assert_eq!(html_to_pango("just plain text"), "just plain text");
        assert_eq!(html_to_pango(""), "");
    }

    #[test]
    fn overlong_input_is_truncated_not_panicking() {
        let huge = "<b>".to_string() + &"x".repeat(MAX_LEN * 2);
        let out = html_to_pango(&huge);
        assert!(out.len() <= MAX_LEN + 16);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// However malformed, this must never panic, and must never emit
        /// an odd number of `<`/`>` that would suggest broken markup --
        /// checked loosely here by round-tripping through Pango's own
        /// (offline, no display needed) markup parser is not available in
        /// this crate, so the practical invariant checked is "no panic,
        /// bounded output".
        #[test]
        fn never_panics_on_arbitrary_html(html in ".{0,2000}") {
            let out = html_to_pango(&html);
            prop_assert!(out.len() <= MAX_LEN + 4096);
        }
    }
}
