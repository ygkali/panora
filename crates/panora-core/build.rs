// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Turns `po/panora.pot` and the translation catalogues next to it into the
//! `Strings` struct and its two instances, so translators work with the
//! files their tools already understand and the binary still carries the
//! strings with no gettext runtime.
//!
//! The template is the source of truth: it fixes the field order, the field
//! names (`msgctxt`), the English text (`msgid`) and the documentation
//! (`#.` comments). A translation that is missing an entry, that leaves one
//! empty or that adds one the template does not have fails the build --
//! there is no runtime fallback to hide behind.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// One `msgctxt` / `msgid` / `msgstr` block.
struct Entry {
    context: String,
    id: String,
    text: String,
    comments: Vec<String>,
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let po_dir = manifest.join("../../po");
    let template = po_dir.join("panora.pot");
    let turkish = po_dir.join("tr.po");

    for path in [&template, &turkish] {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let fields = parse(&template);
    let tr = parse(&turkish)
        .into_iter()
        .map(|entry| (entry.context.clone(), entry))
        .collect::<BTreeMap<_, _>>();

    if fields.is_empty() {
        panic!("{}: no entries", template.display());
    }

    let mut struct_body = String::new();
    let mut english = String::new();
    let mut turkish_body = String::new();

    for field in &fields {
        for comment in &field.comments {
            writeln!(struct_body, "    /// {comment}").unwrap();
        }
        writeln!(struct_body, "    pub {}: &'static str,", field.context).unwrap();
        writeln!(
            english,
            "    {}: \"{}\",",
            field.context,
            rust_escape(&field.id)
        )
        .unwrap();

        let Some(entry) = tr.get(&field.context) else {
            panic!("tr.po has no entry for `{}`", field.context);
        };
        if entry.text.is_empty() {
            panic!("tr.po leaves `{}` untranslated", field.context);
        }
        if entry.id != field.id {
            panic!(
                "tr.po is stale: `{}` was translated against a different \
                 source string; run msgmerge",
                field.context
            );
        }
        writeln!(
            turkish_body,
            "    {}: \"{}\",",
            field.context,
            rust_escape(&entry.text)
        )
        .unwrap();
    }

    for context in tr.keys() {
        if !fields.iter().any(|field| &field.context == context) {
            panic!("tr.po has `{context}`, which the template does not");
        }
    }

    let generated = format!(
        "// Generated from po/panora.pot and po/tr.po by build.rs. Do not edit.\n\
         /// All user-visible strings. Placeholders are documented per field.\n\
         #[allow(missing_docs)]\n\
         #[derive(Debug)]\n\
         pub struct Strings {{\n{struct_body}}}\n\
         \n\
         static EN: Strings = Strings {{\n{english}}};\n\
         \n\
         static TR: Strings = Strings {{\n{turkish_body}}};\n"
    );

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("catalogue.rs");
    std::fs::write(&out, generated).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
}

/// Read a `.po` / `.pot` file. Only what these catalogues use: `msgctxt`,
/// `msgid`, `msgstr`, `#.` comments and continuation lines. The header
/// entry (an empty `msgid` with no context) is skipped.
fn parse(path: &Path) -> Vec<Entry> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut entries = Vec::new();
    let mut comments: Vec<String> = Vec::new();
    let mut context: Option<String> = None;
    let mut id: Option<String> = None;
    let mut text_value: Option<String> = None;
    // Which of the three the continuation lines belong to.
    let mut current = Field::None;

    let flush = |context: &mut Option<String>,
                 id: &mut Option<String>,
                 text_value: &mut Option<String>,
                 comments: &mut Vec<String>,
                 entries: &mut Vec<Entry>| {
        if let (Some(context), Some(id), Some(text)) =
            (context.take(), id.take(), text_value.take())
        {
            entries.push(Entry {
                context,
                id,
                text,
                comments: std::mem::take(comments),
            });
        } else {
            comments.clear();
        }
    };

    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            flush(
                &mut context,
                &mut id,
                &mut text_value,
                &mut comments,
                &mut entries,
            );
            current = Field::None;
            continue;
        }
        if let Some(rest) = line.strip_prefix("#.") {
            comments.push(rest.trim().to_string());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let fail =
            |what: &str| -> ! { panic!("{}:{}: {what}: {line}", path.display(), number + 1) };
        if let Some(rest) = line.strip_prefix("msgctxt ") {
            context = Some(unquote(rest).unwrap_or_else(|| fail("bad msgctxt")));
            current = Field::Context;
        } else if let Some(rest) = line.strip_prefix("msgid ") {
            id = Some(unquote(rest).unwrap_or_else(|| fail("bad msgid")));
            current = Field::Id;
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            text_value = Some(unquote(rest).unwrap_or_else(|| fail("bad msgstr")));
            current = Field::Text;
        } else if line.starts_with('"') {
            let more = unquote(line).unwrap_or_else(|| fail("bad continuation"));
            match current {
                Field::Context => context.as_mut().unwrap().push_str(&more),
                Field::Id => id.as_mut().unwrap().push_str(&more),
                Field::Text => text_value.as_mut().unwrap().push_str(&more),
                Field::None => fail("continuation without a string"),
            }
        } else {
            fail("unexpected line");
        }
    }
    flush(
        &mut context,
        &mut id,
        &mut text_value,
        &mut comments,
        &mut entries,
    );
    entries
}

enum Field {
    None,
    Context,
    Id,
    Text,
}

/// `"a\nb"` -> `a<newline>b`. Returns `None` when the line is not a quoted
/// string.
fn unquote(line: &str) -> Option<String> {
    let inner = line.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    Some(out)
}

/// Escape for a Rust string literal, leaving non-ASCII text alone so the
/// generated file stays readable.
fn rust_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}
