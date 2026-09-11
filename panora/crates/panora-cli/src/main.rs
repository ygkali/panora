// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Low-overhead Panora CLI client.

#![forbid(unsafe_code)]

use panora_core::config::Config;
use panora_core::i18n::{fill, Language, Strings};
use panora_core::ipc::{client, QueryRequest, Request, ResponseData};
use std::io::Write;

/// Parsed command line.
struct Invocation {
    request: Request,
    json: bool,
    /// `preview --mime`: write only this payload.
    mime: Option<String>,
    /// `preview --out`: write the payload to a file instead of stdout.
    out: Option<String>,
}

fn main() {
    let language = Config::load()
        .map(|c| Language::from_config(&c.ui.language))
        .unwrap_or_else(|_| Language::from_environment());
    let s = language.strings();
    if let Err(error) = run(s) {
        eprintln!("panora-cli: {error}");
        std::process::exit(1);
    }
}

fn run(s: &Strings) -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(invocation) = parse(&args, s)? else {
        println!("{}", s.cli_help);
        return Ok(());
    };
    let data = client::call(&invocation.request).map_err(|e| e.to_string())?;
    print_response(s, &invocation, data)
}

fn parse(args: &[String], s: &Strings) -> Result<Option<Invocation>, String> {
    let mut json = false;
    let mut positional: Vec<&str> = Vec::new();
    let mut kind = None;
    let mut pinned = false;
    let mut limit = 50usize;
    let mut offset = 0usize;
    let mut paste = false;
    let mut mime = None;
    let mut out = None;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--pinned" => pinned = true,
            "--paste" => paste = true,
            "--kind" => kind = Some(next_value(&mut iter, "--kind")?),
            "--limit" => {
                limit = next_value(&mut iter, "--limit")?
                    .parse()
                    .map_err(|_| "--limit must be a number")?
            }
            "--offset" => {
                offset = next_value(&mut iter, "--offset")?
                    .parse()
                    .map_err(|_| "--offset must be a number")?
            }
            "--mime" => mime = Some(next_value(&mut iter, "--mime")?),
            "--out" => out = Some(next_value(&mut iter, "--out")?),
            other => positional.push(other),
        }
    }

    let request = match positional.first().copied() {
        None | Some("help") | Some("--help") | Some("-h") => return Ok(None),
        Some("list") => Request::List(QueryRequest {
            search: positional.get(1).map(|q| q.to_string()),
            kind,
            pinned_only: pinned,
            limit,
            offset,
        }),
        Some("search") => Request::List(QueryRequest {
            search: Some(positional.get(1).ok_or("search requires text")?.to_string()),
            kind,
            pinned_only: pinned,
            limit,
            offset,
        }),
        Some("copy") | Some("recall") => Request::Recall {
            id: parse_id(&positional)?,
            paste,
        },
        Some("pin") => Request::Pin {
            id: parse_id(&positional)?,
            pinned: true,
        },
        Some("unpin") => Request::Pin {
            id: parse_id(&positional)?,
            pinned: false,
        },
        Some("delete") | Some("rm") => Request::Delete {
            id: parse_id(&positional)?,
        },
        Some("clear") => Request::Clear,
        Some("private") => Request::SetPrivate {
            enabled: match positional.get(1).copied() {
                Some("on") | Some("1") | Some("true") => true,
                Some("off") | Some("0") | Some("false") => false,
                _ => return Err("private requires on or off".into()),
            },
        },
        Some("status") => Request::Status,
        Some("preview") | Some("show") => Request::Preview {
            id: parse_id(&positional)?,
        },
        Some("toggle") => Request::Toggle,
        Some("reload") => Request::ReloadConfig,
        Some(other) => return Err(format!("{}: {other}", s.cli_unknown_command)),
    };
    Ok(Some(Invocation {
        request,
        json,
        mime,
        out,
    }))
}

fn next_value<'a>(iter: &mut std::slice::Iter<'a, String>, flag: &str) -> Result<String, String> {
    iter.next()
        .map(|v| v.to_string())
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_id(args: &[&str]) -> Result<i64, String> {
    args.get(1)
        .ok_or("command requires an id")?
        .parse()
        .map_err(|_| "id must be an integer".into())
}

fn print_response(s: &Strings, invocation: &Invocation, data: ResponseData) -> Result<(), String> {
    if invocation.json {
        let json = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
        println!("{json}");
        return Ok(());
    }
    match data {
        ResponseData::Entries(entries) => {
            for entry in entries {
                let pin = if entry.pinned { "*" } else { " " };
                let preview = entry.preview.replace('\n', " ⏎ ");
                println!(
                    "{pin} {:>5} [{}] {}",
                    entry.id,
                    entry.kind.as_str(),
                    preview
                );
            }
        }
        ResponseData::Status(status) => {
            let caps = &status.capabilities;
            println!(
                "backend={} entries={} private={} version={} protocol={} revision={} \
                 primary={} persist={} paste={}",
                status.backend,
                status.entries,
                status.private_mode,
                status.version,
                status.protocol,
                status.revision,
                caps.primary,
                caps.persist,
                caps.synthetic_paste
            );
        }
        ResponseData::Count(count) => println!("{}", fill(s.cli_count, "n", &count.to_string())),
        ResponseData::Payloads(payloads) => {
            if let Some(mime) = &invocation.mime {
                let payload = payloads
                    .iter()
                    .find(|p| p.mime.eq_ignore_ascii_case(mime))
                    .ok_or_else(|| format!("no payload with MIME {mime}"))?;
                write_payload(&payload.data, invocation.out.as_deref())?;
            } else if let Some(out) = &invocation.out {
                let payload = payloads.first().ok_or("entry has no payloads")?;
                write_payload(&payload.data, Some(out))?;
            } else {
                for payload in &payloads {
                    if payload.is_text() {
                        println!(
                            "--- {} ({} bytes)\n{}",
                            payload.mime,
                            payload.data.len(),
                            String::from_utf8_lossy(&payload.data)
                        );
                    } else {
                        println!(
                            "--- {} ({} bytes) [binary]",
                            payload.mime,
                            payload.data.len()
                        );
                    }
                }
            }
        }
        ResponseData::Recalled { pasted } => {
            if pasted {
                println!("{} (pasted)", s.cli_ok);
            } else {
                println!("{}", s.cli_ok);
            }
        }
        ResponseData::Empty => println!("{}", s.cli_ok),
    }
    Ok(())
}

fn write_payload(data: &[u8], out: Option<&str>) -> Result<(), String> {
    match out {
        Some(path) => std::fs::write(path, data).map_err(|e| format!("cannot write {path}: {e}")),
        None => {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            lock.write_all(data).map_err(|e| e.to_string())?;
            lock.flush().map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_list_with_options() {
        let s = Language::English.strings();
        let inv = parse(
            &args(&[
                "list", "foo", "--kind", "image", "--pinned", "--limit", "5", "--json",
            ]),
            s,
        )
        .unwrap()
        .unwrap();
        assert!(inv.json);
        match inv.request {
            Request::List(q) => {
                assert_eq!(q.search.as_deref(), Some("foo"));
                assert_eq!(q.kind.as_deref(), Some("image"));
                assert!(q.pinned_only);
                assert_eq!(q.limit, 5);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_copy_with_paste_and_preview_options() {
        let s = Language::Turkish.strings();
        let inv = parse(&args(&["copy", "7", "--paste"]), s).unwrap().unwrap();
        assert!(matches!(
            inv.request,
            Request::Recall { id: 7, paste: true }
        ));
        let inv = parse(
            &args(&["preview", "3", "--mime", "image/png", "--out", "x.png"]),
            s,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(inv.request, Request::Preview { id: 3 }));
        assert_eq!(inv.mime.as_deref(), Some("image/png"));
        assert_eq!(inv.out.as_deref(), Some("x.png"));
    }

    #[test]
    fn rejects_bad_input() {
        let s = Language::English.strings();
        assert!(parse(&args(&["copy", "x"]), s).is_err());
        assert!(parse(&args(&["private", "maybe"]), s).is_err());
        assert!(parse(&args(&["bogus"]), s).is_err());
        assert!(parse(&args(&["--limit"]), s).is_err());
        assert!(parse(&args(&[]), s).unwrap().is_none());
        assert!(matches!(
            parse(&args(&["reload"]), s).unwrap().unwrap().request,
            Request::ReloadConfig
        ));
    }
}
