// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Low-overhead Panora CLI client.

use panora_core::config::socket_path;
use panora_core::model::Entry;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

#[derive(Debug, Serialize)]
#[serde(tag = "method", content = "params")]
enum Request {
    List(QueryRequest),
    Recall { id: i64 },
    Pin { id: i64, pinned: bool },
    Delete { id: i64 },
    Clear,
    SetPrivate { enabled: bool },
    Toggle,
    Status,
    Preview { id: i64 },
}

#[derive(Debug, Serialize, Default)]
struct QueryRequest {
    search: Option<String>,
    kind: Option<String>,
    pinned_only: bool,
    limit: usize,
    offset: usize,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "ok", content = "data")]
enum Response {
    #[serde(rename = "true")]
    Success(ResponseData),
    #[serde(rename = "false")]
    Failure { message: String },
}

#[derive(Debug, Deserialize)]
enum ResponseData {
    Entries(Vec<Entry>),
    Count(usize),
    Status(StatusData),
    Empty,
    Payloads(Vec<panora_core::model::MimePayload>),
}

#[derive(Debug, Deserialize)]
struct StatusData {
    backend: String,
    entries: i64,
    private_mode: bool,
    sync_active: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("panora-cli: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let request = match args.first().map(String::as_str) {
        None | Some("help") | Some("--help") => {
            print_help();
            return Ok(());
        }
        Some("list") => Request::List(QueryRequest {
            search: args.get(1).cloned(),
            limit: 50,
            ..Default::default()
        }),
        Some("search") => Request::List(QueryRequest {
            search: Some(args.get(1).ok_or("search requires text")?.clone()),
            limit: 50,
            ..Default::default()
        }),
        Some("copy") => Request::Recall {
            id: parse_id(&args)?,
        },
        Some("pin") => Request::Pin {
            id: parse_id(&args)?,
            pinned: true,
        },
        Some("unpin") => Request::Pin {
            id: parse_id(&args)?,
            pinned: false,
        },
        Some("delete") => Request::Delete {
            id: parse_id(&args)?,
        },
        Some("clear") => Request::Clear,
        Some("private") => Request::SetPrivate {
            enabled: match args.get(1).map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => return Err("private requires on or off".into()),
            },
        },
        Some("status") => Request::Status,
        Some("preview") => Request::Preview {
            id: parse_id(&args)?,
        },
        Some("toggle") => Request::Toggle,
        Some(other) => return Err(format!("unknown command: {other}")),
    };
    let response = call(request)?;
    print_response(response);
    Ok(())
}

fn parse_id(args: &[String]) -> Result<i64, String> {
    args.get(1)
        .ok_or("command requires an id")?
        .parse()
        .map_err(|_| "id must be an integer".into())
}

fn call(request: Request) -> Result<Response, String> {
    let mut stream =
        UnixStream::connect(socket_path()).map_err(|e| format!("daemon unavailable: {e}"))?;
    let bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(line.trim()).map_err(|e| format!("invalid daemon response: {e}"))
}

fn print_response(response: Response) {
    match response {
        Response::Failure { message } => eprintln!("error: {message}"),
        Response::Success(data) => match data {
            ResponseData::Entries(entries) => {
                for entry in entries {
                    let pin = if entry.pinned { "*" } else { " " };
                    println!(
                        "{pin} {:>4} [{}] {}",
                        entry.id,
                        entry.kind.as_str(),
                        entry.preview
                    );
                }
            }
            ResponseData::Status(status) => println!(
                "backend={} entries={} private={} sync={}",
                status.backend, status.entries, status.private_mode, status.sync_active
            ),
            ResponseData::Count(count) => println!("{count} kayıt işlendi."),
            ResponseData::Payloads(payloads) => {
                for payload in payloads {
                    println!("{} {} bytes", payload.mime, payload.data.len());
                }
            }
            ResponseData::Empty => println!("ok"),
        },
    }
}

fn print_help() {
    println!("Panora güvenli pano yöneticisi CLI");
    println!("Kullanım:");
    println!("  panora-cli list [arama]");
    println!("  panora-cli search <metin>");
    println!("  panora-cli copy|preview <id>");
    println!("  panora-cli pin|unpin <id>");
    println!("  panora-cli delete <id>");
    println!("  panora-cli clear");
    println!("  panora-cli private on|off");
    println!("  panora-cli status");
    println!("  panora-cli toggle");
}
