// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Command-line client for the Panora clipboard daemon.
//!
//! Every command is one request over the daemon's private Unix socket
//! (`panora_core::ipc`), so encryption, retention and the privacy gates apply
//! exactly as they do for the popup. The parser is `clap`, which also renders
//! the man pages and shell completions the Debian package ships.

#![forbid(unsafe_code)]

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use panora_core::config::Config;
use panora_core::error::Error;
use panora_core::i18n::{fill, Language, Strings};
use panora_core::ipc::{client, QueryRequest, Request, ResponseData, MAX_FRAME_BYTES};
use panora_core::model::{Entry, MimePayload};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

/// Exit statuses, documented in `--help` and the man page. `2` is clap's
/// usage error and is not listed here.
mod exit {
    pub const FAILURE: u8 = 1;
    pub const NO_DAEMON: u8 = 3;
    pub const NOT_FOUND: u8 = 4;
}

#[derive(Parser, Debug)]
#[command(
    name = "panora-cli",
    version,
    about = "Command-line client for the Panora clipboard daemon",
    long_about = "Talks to the local panod daemon over its private Unix socket. Every command \
                  that changes the history goes through the daemon, so encryption, retention \
                  and the privacy rules apply exactly as they do for the popup.",
    after_help = "Exit status:\n  0  success\n  1  the daemon reported an error\n  2  usage \
                  error\n  3  the daemon is not running\n  4  no entry with that id",
    propagate_version = true
)]
struct Cli {
    /// Print machine-readable JSON instead of text
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List recent entries, newest first (pinned entries come first)
    List {
        /// Search string; every word is a prefix. Operators: kind:image app:firefox
        /// pinned:yes after:7d before:2026-09-01 "exact phrase"; re:PATTERN matches a
        /// regular expression against the text
        query: Option<String>,
        /// Line template instead of the default listing (see `pick --help`)
        #[arg(long, value_name = "TEMPLATE")]
        format: Option<String>,
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// Full-text search of the history
    Search {
        /// Search string; every word is a prefix ("mer" finds "merhaba"). Operators:
        /// kind:image app:firefox pinned:yes after:7d before:2026-09-01 "exact phrase";
        /// re:PATTERN matches a regular expression against the text
        text: String,
        /// Line template instead of the default listing (see `pick --help`)
        #[arg(long, value_name = "TEMPLATE")]
        format: Option<String>,
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// Put an entry back on the clipboard
    #[command(alias = "recall")]
    Copy {
        /// Entry id as shown by `list`
        id: i64,
        /// Also send a paste keystroke to the focused window
        #[arg(long)]
        paste: bool,
        /// Offer only this format (e.g. text/plain to drop the HTML of a rich-text entry)
        #[arg(long, value_name = "TYPE")]
        mime: Option<String>,
    },
    /// Print or export the decrypted payloads of an entry
    #[command(alias = "show")]
    Preview {
        /// Entry id
        id: i64,
        /// Write only this format
        #[arg(long, value_name = "TYPE")]
        mime: Option<String>,
        /// Write the payload to this file instead of standard output
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Pin an entry so retention and `clear` keep it
    Pin {
        /// Entry id
        id: i64,
    },
    /// Remove the pin from an entry
    Unpin {
        /// Entry id
        id: i64,
    },
    /// Delete one entry and its payloads
    #[command(alias = "rm")]
    Delete {
        /// Entry id
        id: i64,
    },
    /// Delete every unpinned entry
    Clear,
    /// Pause or resume recording (private mode)
    Private {
        /// on to pause recording, off to resume
        state: OnOff,
    },
    /// Show daemon health, backend capabilities and history size
    Status,
    /// Show or hide the popup
    Toggle,
    /// Re-read config.toml and apply it without restarting the daemon
    Reload,
    /// Generate a new master key and reseal the whole history under it
    ///
    /// Safe to interrupt (Ctrl+C, a crash, a daemon restart): history stays
    /// fully readable throughout, and running this again finishes the same
    /// rotation instead of starting a new one. `panora-cli status` reports
    /// `rotation_incomplete` under health if a previous attempt was left
    /// unfinished.
    RotateKey,
    /// Engage the second-layer lock, or manage its password (SEC-02)
    ///
    /// While engaged: `list`/`search` report a count only, `preview` and
    /// `copy`/`recall` are refused. Passwords are always read from
    /// standard input, one per line, never as a command-line argument
    /// (shell history, `ps` would show it).
    Lock {
        #[command(subcommand)]
        action: Option<LockAction>,
    },
    /// Disengage the second-layer lock
    ///
    /// Reads the password from standard input.
    Unlock,
    /// Bring back an entry deleted in the last 30 seconds
    Restore {
        /// Entry id
        id: i64,
    },
    /// Record text from a file or standard input as a new entry
    Store {
        /// File to read; "-" or nothing reads standard input (48 KiB at most)
        file: Option<PathBuf>,
        /// MIME type of the content
        #[arg(long, default_value = "text/plain;charset=utf-8", value_name = "TYPE")]
        mime: String,
        /// Application name the entry is attributed to
        #[arg(long, value_name = "NAME")]
        app: Option<String>,
        /// Only record it; do not put it on the clipboard
        #[arg(long)]
        no_copy: bool,
    },
    /// Print entries one per line for dmenu, rofi, fuzzel or wofi
    ///
    /// Example: panora-cli pick | fuzzel --dmenu | cut -f1 | xargs panora-cli copy --paste
    Pick {
        /// Line template: {id} {kind} {preview} {app} {age} {size} {pinned} {sensitive}; \t and \n are escapes
        #[arg(
            long,
            default_value = "{id}\\t{kind}\\t{preview}",
            value_name = "TEMPLATE"
        )]
        format: String,
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// Print one JSON line per history change until interrupted (Ctrl+C)
    ///
    /// The first line is the daemon's current revision, so a watcher never
    /// misses a change that happened just before it connected. Text mode
    /// prints `revision=N`; `--json` prints the same shape `status`'s JSON
    /// output uses for its `revision` field. Compose with `list`/`search`:
    /// `panora-cli watch | while read -r _; do panora-cli list; done`
    Watch,
    /// Print a shell completion script to standard output
    Completions {
        /// Shell to generate for
        shell: Shell,
    },
    /// Write the man pages into a directory (used by the package build)
    #[command(hide = true)]
    Man {
        /// Output directory
        dir: PathBuf,
    },
}

/// Filters shared by `list` and `search`.
#[derive(Args, Debug, Default)]
struct FilterArgs {
    /// Only entries of this kind
    #[arg(long, value_enum)]
    kind: Option<Kind>,
    /// Only pinned entries
    #[arg(long)]
    pinned: bool,
    /// Page size (at most 500)
    #[arg(long, default_value_t = 50, value_name = "N")]
    limit: usize,
    /// Skip this many entries
    #[arg(long, default_value_t = 0, value_name = "N")]
    offset: usize,
}

/// Content kinds accepted by `--kind`; the names match `ContentKind::as_str`.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Text,
    Richtext,
    Link,
    Image,
    Files,
    Color,
    Binary,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Richtext => "richtext",
            Kind::Link => "link",
            Kind::Image => "image",
            Kind::Files => "files",
            Kind::Color => "color",
            Kind::Binary => "binary",
        }
    }
}

/// `private on|off`, also accepting the usual boolean spellings.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum OnOff {
    #[value(aliases = ["1", "true", "yes"])]
    On,
    #[value(aliases = ["0", "false", "no"])]
    Off,
}

/// `lock` subcommands that manage the password itself (SEC-02), as opposed
/// to `lock` bare (engage) and `unlock` (disengage).
#[derive(Subcommand, Debug)]
enum LockAction {
    /// Set the lock password; only valid when none is set yet. Reads the
    /// new password from standard input.
    #[command(name = "set-password")]
    Set,
    /// Change the existing lock password. Reads two lines from standard
    /// input: the current password, then the new one.
    #[command(name = "change-password")]
    Change,
    /// Remove the lock password (this also disengages the lock). Reads the
    /// current password from standard input to verify.
    #[command(name = "remove-password")]
    Remove,
}

/// What a command needs from the daemon and how its reply is shown.
struct Invocation {
    request: Request,
    /// `preview --mime`: write only this payload.
    mime: Option<String>,
    /// `preview --out`: write the payload to a file instead of stdout.
    out: Option<PathBuf>,
    /// `list`/`search`/`pick --format`: one line per entry from a template.
    format: Option<String>,
}

/// Largest payload `store` accepts: it travels base64-encoded inside one
/// request frame of `MAX_FRAME_BYTES`, with room for the JSON around it.
const STORE_MAX_BYTES: usize = MAX_FRAME_BYTES / 4 * 3 - 1024;

/// `rotate-key` reseals the whole history before replying; generous rather
/// than tuned to any particular history size.
const ROTATE_KEY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// A failure with the exit status it maps to.
struct Failure {
    code: u8,
    message: String,
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        let message = error.to_string();
        let code = match &error {
            Error::Ipc(m) if m.starts_with("daemon unavailable") => exit::NO_DAEMON,
            Error::Ipc(m) if m.starts_with("entry not found") => exit::NOT_FOUND,
            Error::NotFound(_) => exit::NOT_FOUND,
            _ => exit::FAILURE,
        };
        Self { code, message }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            code: exit::FAILURE,
            message,
        }
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let language = Config::load()
        .map(|c| Language::from_config(&c.ui.language))
        .unwrap_or_else(|_| Language::from_environment());
    let s = language.strings();
    match run(cli, s) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("panora-cli: {}", failure.message);
            ExitCode::from(failure.code)
        }
    }
}

fn run(cli: Cli, s: &Strings) -> Result<(), Failure> {
    let json = cli.json;
    let invocation = match cli.command {
        Command::Completions { shell } => {
            let mut command = Cli::command();
            clap_complete::generate(shell, &mut command, "panora-cli", &mut std::io::stdout());
            return Ok(());
        }
        Command::Man { dir } => {
            write_man_pages(&dir).map_err(|e| format!("cannot write man pages: {e}"))?;
            return Ok(());
        }
        Command::Watch => return run_watch(json),
        Command::RotateKey => {
            // Reseals the whole history inline before replying; 5s (the
            // default IPC timeout) would misreport a still-working daemon
            // as hung on anything but a tiny history.
            let data = client::call_with_timeout(&Request::RotateKey, ROTATE_KEY_TIMEOUT)?;
            return print_response(
                s,
                json,
                &Invocation {
                    request: Request::RotateKey,
                    mime: None,
                    out: None,
                    format: None,
                },
                data,
            );
        }
        Command::Unlock => {
            let password = read_password_line()?;
            let request = Request::Unlock { password };
            let data = client::call(&request)?;
            return print_response(s, json, &no_reply_shape(request), data);
        }
        Command::Lock { action: None } => {
            let data = client::call(&Request::Lock)?;
            return print_response(s, json, &no_reply_shape(Request::Lock), data);
        }
        Command::Lock {
            action: Some(LockAction::Set),
        } => {
            let request = Request::SetLockPassword {
                new_password: Some(read_password_line()?),
                current_password: None,
            };
            let data = client::call(&request)?;
            return print_response(s, json, &no_reply_shape(request), data);
        }
        Command::Lock {
            action: Some(LockAction::Change),
        } => {
            let current_password = read_password_line()?;
            let request = Request::SetLockPassword {
                new_password: Some(read_password_line()?),
                current_password: Some(current_password),
            };
            let data = client::call(&request)?;
            return print_response(s, json, &no_reply_shape(request), data);
        }
        Command::Lock {
            action: Some(LockAction::Remove),
        } => {
            let request = Request::SetLockPassword {
                new_password: None,
                current_password: Some(read_password_line()?),
            };
            let data = client::call(&request)?;
            return print_response(s, json, &no_reply_shape(request), data);
        }
        Command::Store {
            file,
            mime,
            app,
            no_copy,
        } => {
            let data = read_store_input(file.as_deref())?;
            Invocation {
                request: Request::Store {
                    payloads: vec![MimePayload::new(mime, data)],
                    source_app: app,
                    copy: !no_copy,
                },
                mime: None,
                out: None,
                format: None,
            }
        }
        other => to_invocation(other),
    };
    let data = client::call(&invocation.request)?;
    print_response(s, json, &invocation, data)
}

/// Map a parsed command onto the IPC request it stands for.
fn to_invocation(command: Command) -> Invocation {
    let plain = |request| Invocation {
        request,
        mime: None,
        out: None,
        format: None,
    };
    let listing = |request, format| Invocation {
        request,
        mime: None,
        out: None,
        format,
    };
    match command {
        Command::List {
            query,
            format,
            filter,
        } => listing(Request::List(query_request(query, filter)), format),
        Command::Search {
            text,
            format,
            filter,
        } => listing(Request::List(query_request(Some(text), filter)), format),
        Command::Pick { format, filter } => {
            listing(Request::List(query_request(None, filter)), Some(format))
        }
        Command::Copy { id, paste, mime } => plain(Request::Recall { id, paste, mime }),
        Command::Preview { id, mime, out } => Invocation {
            request: Request::Preview {
                id,
                thumbnail: false,
            },
            mime,
            out,
            format: None,
        },
        Command::Restore { id } => plain(Request::Restore { id }),
        Command::Pin { id } => plain(Request::Pin { id, pinned: true }),
        Command::Unpin { id } => plain(Request::Pin { id, pinned: false }),
        Command::Delete { id } => plain(Request::Delete { id }),
        Command::Clear => plain(Request::Clear),
        Command::Private { state } => plain(Request::SetPrivate {
            enabled: state == OnOff::On,
        }),
        Command::Status => plain(Request::Status),
        Command::Toggle => plain(Request::Toggle),
        Command::Reload => plain(Request::ReloadConfig),
        Command::Completions { .. }
        | Command::Man { .. }
        | Command::Store { .. }
        | Command::Watch
        | Command::RotateKey
        | Command::Lock { .. }
        | Command::Unlock => {
            unreachable!("handled before reaching the daemon")
        }
    }
}

/// An `Invocation` for a request whose only successful reply is `Empty`
/// (the `lock`/`unlock` family): `mime`/`out`/`format` are never read for
/// it, but `request` stays the real one for consistency with every other
/// `Invocation`.
fn no_reply_shape(request: Request) -> Invocation {
    Invocation {
        request,
        mime: None,
        out: None,
        format: None,
    }
}

/// Read one line of secret input (a lock password) from standard input,
/// trimmed of its line ending. Never a command-line argument: that would
/// put it in the shell history and be visible to any process reading
/// `/proc/<pid>/cmdline` (`ps`).
fn read_password_line() -> Result<String, Failure> {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("cannot read the password from standard input: {e}"))?;
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return Err("no password given on standard input".into());
    }
    Ok(line.to_string())
}

/// Read the content for `store` from a file or standard input, bounded by
/// what one IPC frame can carry.
fn read_store_input(file: Option<&std::path::Path>) -> Result<Vec<u8>, Failure> {
    use std::io::Read as _;
    let mut data = Vec::new();
    match file {
        Some(path) if path != std::path::Path::new("-") => {
            let handle = std::fs::File::open(path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            std::io::Read::take(handle, STORE_MAX_BYTES as u64 + 1)
                .read_to_end(&mut data)
                .map_err(|e| e.to_string())?;
        }
        _ => {
            std::io::stdin()
                .lock()
                .take(STORE_MAX_BYTES as u64 + 1)
                .read_to_end(&mut data)
                .map_err(|e| e.to_string())?;
        }
    }
    if data.len() > STORE_MAX_BYTES {
        return Err(format!(
            "store accepts at most {} bytes (one IPC frame); copy larger content from an application",
            STORE_MAX_BYTES
        )
        .into());
    }
    if data.is_empty() {
        return Err("nothing to store".into());
    }
    Ok(data)
}

/// `watch`: block on `Subscribe` (STO-08) and print one line per event
/// until the daemon closes the connection or the process is interrupted.
fn run_watch(json: bool) -> Result<(), Failure> {
    let subscription = client::Subscription::open()?;
    let stdout = std::io::stdout();
    loop {
        let event = subscription.next()?;
        let mut out = stdout.lock();
        if json {
            let line = serde_json::to_string(&event).map_err(|e| e.to_string())?;
            writeln!(out, "{line}").map_err(|e| e.to_string())?;
        } else {
            let panora_core::ipc::Event::Changed { revision } = event;
            writeln!(out, "revision={revision}").map_err(|e| e.to_string())?;
        }
        out.flush().map_err(|e| e.to_string())?;
    }
}

/// Expand a `pick`/`--format` template for one entry.
fn format_entry(template: &str, entry: &Entry, now: i64) -> String {
    let preview = entry.preview.replace('\n', " ⏎ ");
    let age = {
        let seconds = (now - entry.last_seen_at).max(0);
        if seconds < 60 {
            format!("{seconds}s")
        } else if seconds < 3600 {
            format!("{}m", seconds / 60)
        } else if seconds < 86_400 {
            format!("{}h", seconds / 3600)
        } else {
            format!("{}d", seconds / 86_400)
        }
    };
    template
        .replace("\\t", "\t")
        .replace("\\n", "\n")
        .replace("{id}", &entry.id.to_string())
        .replace("{kind}", entry.kind.as_str())
        .replace("{preview}", &preview)
        .replace("{app}", entry.source_app.as_deref().unwrap_or(""))
        .replace("{age}", &age)
        .replace("{size}", &entry.size_bytes.to_string())
        .replace("{pinned}", if entry.pinned { "*" } else { "" })
        .replace("{sensitive}", if entry.sensitive { "!" } else { "" })
}

fn query_request(search: Option<String>, filter: FilterArgs) -> QueryRequest {
    QueryRequest {
        search: search.filter(|q| !q.trim().is_empty()),
        kind: filter.kind.map(|k| k.as_str().to_string()),
        pinned_only: filter.pinned,
        limit: filter.limit,
        offset: filter.offset,
    }
}

/// Render `panora-cli.1` plus one page per subcommand into `dir`.
fn write_man_pages(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let root = Cli::command();
    let render = |command: clap::Command, file: &str| -> std::io::Result<()> {
        let mut buffer = Vec::new();
        clap_mangen::Man::new(command).render(&mut buffer)?;
        std::fs::write(dir.join(file), buffer)
    };
    render(root.clone(), "panora-cli.1")?;
    for sub in root.get_subcommands().filter(|c| !c.is_hide_set()) {
        let name = format!("panora-cli-{}", sub.get_name());
        render(sub.clone().name(name.clone()), &format!("{name}.1"))?;
    }
    Ok(())
}

fn print_response(
    s: &Strings,
    json: bool,
    invocation: &Invocation,
    data: ResponseData,
) -> Result<(), Failure> {
    if json {
        let json = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
        println!("{json}");
        return Ok(());
    }
    match data {
        ResponseData::Entries(entries) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            for entry in entries {
                if let Some(template) = &invocation.format {
                    println!("{}", format_entry(template, &entry, now));
                    continue;
                }
                let pin = if entry.pinned {
                    "*"
                } else if entry.sensitive {
                    "!"
                } else {
                    " "
                };
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
                "backend={} entries={} private={} locked={} app_locked={} \
                 lock_password_set={} version={} protocol={} revision={} primary={} \
                 persist={} paste={} source_app={} needs_bridge={}",
                status.backend,
                status.entries,
                status.private_mode,
                status.locked,
                status.app_locked,
                status.lock_password_set,
                status.version,
                status.protocol,
                status.revision,
                caps.primary,
                caps.persist,
                caps.synthetic_paste,
                caps.source_app,
                caps.needs_bridge
            );
            for item in &status.health {
                println!("health={} {}", item.code, item.message);
            }
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
        // Only ever produced on a v3 connection's own `Hello` negotiation,
        // which `client::call` (v2) never sends; kept for exhaustiveness.
        ResponseData::Hello { protocol } => println!("protocol={protocol}"),
    }
    Ok(())
}

fn write_payload(data: &[u8], out: Option<&std::path::Path>) -> Result<(), Failure> {
    match out {
        Some(path) => std::fs::write(path, data)
            .map_err(|e| format!("cannot write {}: {e}", path.display()).into()),
        None => {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            lock.write_all(data).map_err(|e| e.to_string())?;
            lock.flush().map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("panora-cli").chain(args.iter().copied()))
    }

    #[test]
    fn command_line_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_list_with_options() {
        let cli = parse(&[
            "list", "foo", "--kind", "image", "--pinned", "--limit", "5", "--json",
        ])
        .unwrap();
        assert!(cli.json);
        match to_invocation(cli.command).request {
            Request::List(q) => {
                assert_eq!(q.search.as_deref(), Some("foo"));
                assert_eq!(q.kind.as_deref(), Some("image"));
                assert!(q.pinned_only);
                assert_eq!(q.limit, 5);
                assert_eq!(q.offset, 0);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_copy_with_paste_and_preview_options() {
        let cli = parse(&["copy", "7", "--paste"]).unwrap();
        assert!(matches!(
            to_invocation(cli.command).request,
            Request::Recall {
                id: 7,
                paste: true,
                mime: None
            }
        ));
        let cli = parse(&["recall", "8", "--mime", "text/plain"]).unwrap();
        assert!(matches!(
            to_invocation(cli.command).request,
            Request::Recall { id: 8, paste: false, mime: Some(m) } if m == "text/plain"
        ));
        let cli = parse(&["preview", "3", "--mime", "image/png", "--out", "x.png"]).unwrap();
        let inv = to_invocation(cli.command);
        assert!(matches!(inv.request, Request::Preview { id: 3, .. }));
        assert_eq!(inv.mime.as_deref(), Some("image/png"));
        assert_eq!(inv.out.as_deref(), Some(std::path::Path::new("x.png")));
    }

    #[test]
    fn private_accepts_boolean_spellings() {
        for (word, expected) in [("on", true), ("1", true), ("off", false), ("false", false)] {
            let cli = parse(&["private", word]).unwrap();
            assert!(matches!(
                to_invocation(cli.command).request,
                Request::SetPrivate { enabled } if enabled == expected
            ));
        }
        assert!(parse(&["private", "maybe"]).is_err());
    }

    #[test]
    fn simple_commands_map_to_requests() {
        assert!(matches!(
            to_invocation(parse(&["status"]).unwrap().command).request,
            Request::Status
        ));
        assert!(matches!(
            to_invocation(parse(&["toggle"]).unwrap().command).request,
            Request::Toggle
        ));
        assert!(matches!(
            to_invocation(parse(&["reload"]).unwrap().command).request,
            Request::ReloadConfig
        ));
        assert!(matches!(
            to_invocation(parse(&["clear"]).unwrap().command).request,
            Request::Clear
        ));
        assert!(matches!(
            to_invocation(parse(&["rm", "4"]).unwrap().command).request,
            Request::Delete { id: 4 }
        ));
        assert!(matches!(
            to_invocation(parse(&["unpin", "4"]).unwrap().command).request,
            Request::Pin {
                id: 4,
                pinned: false
            }
        ));
    }

    #[test]
    fn pick_and_format_render_templates() {
        let cli = parse(&["pick"]).unwrap();
        let inv = to_invocation(cli.command);
        assert_eq!(inv.format.as_deref(), Some("{id}\\t{kind}\\t{preview}"));
        let cli = parse(&["list", "--format", "{id}: {preview} ({age}, {pinned})"]).unwrap();
        let inv = to_invocation(cli.command);
        let entry = Entry {
            id: 7,
            content_hash: String::new(),
            preview: "two\nlines".into(),
            kind: panora_core::model::ContentKind::Text,
            primary_mime: "text/plain".into(),
            size_bytes: 9,
            source_app: Some("firefox".into()),
            created_at: 0,
            last_seen_at: 1_000,
            pinned: true,
            sensitive: false,
            selection: panora_core::model::Selection::Clipboard,
            device_id: String::new(),
            lamport: 1,
            deleted: false,
        };
        assert_eq!(
            format_entry(inv.format.as_deref().unwrap(), &entry, 1_000 + 3 * 3600),
            "7: two ⏎ lines (3h, *)"
        );
        assert_eq!(
            format_entry("{id}\\t{app}\\t{size}", &entry, 1_000),
            "7\tfirefox\t9"
        );
        assert!(matches!(
            to_invocation(parse(&["restore", "3"]).unwrap().command).request,
            Request::Restore { id: 3 }
        ));
        let cli = parse(&[
            "store",
            "--mime",
            "text/plain",
            "--app",
            "script",
            "--no-copy",
            "-",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Store { no_copy: true, .. }));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&["copy", "x"]).is_err());
        assert!(parse(&["bogus"]).is_err());
        assert!(parse(&["list", "--limit"]).is_err());
        assert!(parse(&[]).is_err(), "a subcommand is required");
        assert!(parse(&["search"]).is_err(), "search needs text");
    }

    #[test]
    fn failures_map_to_documented_exit_codes() {
        let no_daemon = Failure::from(Error::Ipc("daemon unavailable: no socket".into()));
        assert_eq!(no_daemon.code, exit::NO_DAEMON);
        let missing = Failure::from(Error::Ipc("entry not found: 9".into()));
        assert_eq!(missing.code, exit::NOT_FOUND);
        let other = Failure::from(Error::Ipc("IPC request limit exceeded".into()));
        assert_eq!(other.code, exit::FAILURE);
        assert_eq!(Failure::from("plain").code, exit::FAILURE);
    }

    #[test]
    fn man_pages_render_for_every_visible_command() {
        let dir = std::env::temp_dir().join(format!("panora-cli-man-{}", std::process::id()));
        write_man_pages(&dir).unwrap();
        let mut pages: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        pages.sort();
        assert!(pages.contains(&"panora-cli.1".to_string()));
        assert!(pages.contains(&"panora-cli-list.1".to_string()));
        assert!(!pages.contains(&"panora-cli-man.1".to_string()), "hidden");
        let main_page = std::fs::read_to_string(dir.join("panora-cli.1")).unwrap();
        assert!(main_page.contains("Exit status"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
