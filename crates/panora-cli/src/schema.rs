// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Versioned envelope and JSON Schema for `--json` output (CLI-08).
//!
//! Every `--json` reply -- one-shot or one line of `watch --json` -- is
//! `{"schema_version": N, "data": ...}` instead of the bare daemon reply, so
//! a script can tell which shape it is reading without guessing from the
//! command name. `panora-cli schema` prints the JSON Schema (draft
//! 2020-12) for `data` in every command, generated with `schemars` from the
//! same Rust types that are actually serialized -- not a hand-written copy
//! that could quietly drift from the real output.

use panora_core::config::Config;
use panora_core::ipc::{Event, ResponseData};
use schemars::generate::SchemaSettings;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{json, Map, Value};

/// `panora-cli`'s own JSON output schema version. Bumped only when an
/// existing field's meaning changes or a field disappears; a new
/// `ResponseData`/`Config`/command is not a breaking change and does not
/// need a bump. Distinct from `panora_core::ipc::PROTOCOL_VERSION` (the
/// wire protocol between clients and the daemon) and from the package
/// `version` reported by `status` -- all three can move independently.
pub const JSON_SCHEMA_VERSION: u32 = 1;

/// The envelope every `--json` reply is wrapped in.
#[derive(Serialize)]
struct Envelope<'a, T: Serialize> {
    schema_version: u32,
    data: &'a T,
}

/// Serialize `value` as a pretty-printed, versioned `--json` reply.
pub fn to_pretty<T: Serialize>(value: &T) -> Result<String, String> {
    let envelope = Envelope {
        schema_version: JSON_SCHEMA_VERSION,
        data: value,
    };
    serde_json::to_string_pretty(&envelope).map_err(|e| e.to_string())
}

/// Serialize `value` as one compact-line, versioned `--json` reply; used by
/// `watch --json`, which prints one object per event rather than one for
/// the whole invocation.
pub fn to_line<T: Serialize>(value: &T) -> Result<String, String> {
    let envelope = Envelope {
        schema_version: JSON_SCHEMA_VERSION,
        data: value,
    };
    serde_json::to_string(&envelope).map_err(|e| e.to_string())
}

/// `config set`'s `--json` reply: the key that was written and the raw
/// string value it was given (the same shape `config_command` always
/// printed, now just versioned).
#[derive(Serialize, JsonSchema)]
pub struct ConfigSetResult {
    pub key: String,
    pub value: String,
}

/// `config validate`'s `--json` reply.
#[derive(Serialize, JsonSchema)]
pub struct ConfigValidateResult {
    pub valid: bool,
}

/// `export`'s `--json` reply.
#[derive(Serialize, JsonSchema)]
pub struct ExportResult {
    pub bytes: u64,
}

/// `import` and `import-legacy`'s `--json` reply.
#[derive(Serialize, JsonSchema)]
pub struct ImportResult {
    pub imported: usize,
}

/// Build the document `panora-cli schema` prints: one JSON Schema per Rust
/// type that can appear as `data`, under `$defs`, plus a `commands` map
/// from subcommand name to the `$defs` entry its `--json` output's `data`
/// field matches.
///
/// Every command that talks to the daemon (`list` through `wipe`) points at
/// `ResponseData` itself rather than at one hand-picked variant: that is
/// genuinely what gets serialized -- whichever variant the daemon actually
/// returned for that request -- so pointing at the full `oneOf` is accurate,
/// not an approximation kept in sync by hand.
pub fn document() -> Value {
    let settings = SchemaSettings::draft2020_12();
    let mut generator = settings.into_generator();
    let response_data = generator.subschema_for::<ResponseData>();
    let event = generator.subschema_for::<Event>();
    let config = generator.subschema_for::<Config>();
    let config_set = generator.subschema_for::<ConfigSetResult>();
    let config_validate = generator.subschema_for::<ConfigValidateResult>();
    let export = generator.subschema_for::<ExportResult>();
    let import = generator.subschema_for::<ImportResult>();
    let defs = generator.take_definitions(false);

    const DAEMON_COMMANDS: &[&str] = &[
        "list",
        "search",
        "pick",
        "copy",
        "preview",
        "pin",
        "unpin",
        "delete",
        "restore",
        "clear",
        "private",
        "status",
        "stats",
        "toggle",
        "reload",
        "rotate-key",
        "lock",
        "unlock",
        "wipe",
    ];
    let mut commands = Map::new();
    for name in DAEMON_COMMANDS {
        commands.insert((*name).into(), to_value(&response_data));
    }
    commands.insert("watch".into(), to_value(&event));
    commands.insert("config get".into(), to_value(&config));
    commands.insert(
        "config get KEY".into(),
        json!({
            "description": "Any JSON value: whatever the TOML value at that \
                dotted config.toml path converts to (string, number, bool \
                or array of strings)."
        }),
    );
    commands.insert("config set".into(), to_value(&config_set));
    commands.insert("config validate".into(), to_value(&config_validate));
    commands.insert("export".into(), to_value(&export));
    commands.insert("import".into(), to_value(&import));
    commands.insert("import-legacy".into(), to_value(&import));

    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "panora-cli --json output",
        "description": "Every `--json` reply is `{\"schema_version\": N, \
            \"data\": ...}`. `schema_version` is this document's own \
            version (see `schema_version` below); `commands` maps each \
            subcommand to the schema its `data` field matches; `$defs` \
            holds the underlying type schemas.",
        "schema_version": JSON_SCHEMA_VERSION,
        "commands": commands,
        "$defs": defs,
    })
}

fn to_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("schemars output is always representable as JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_carries_schema_version_and_data() {
        let json = to_pretty(&ResponseData::Count(3)).unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["schema_version"], JSON_SCHEMA_VERSION);
        assert_eq!(value["data"], json!({"Count": 3}));
    }

    #[test]
    fn line_envelope_is_one_line() {
        let line = to_line(&Event::Changed { revision: 7 }).unwrap();
        assert!(!line.contains('\n'));
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["schema_version"], JSON_SCHEMA_VERSION);
        assert_eq!(value["data"]["event"], "changed");
        assert_eq!(value["data"]["revision"], 7);
    }

    #[test]
    fn document_has_every_command_and_every_def_resolves() {
        let doc = document();
        let commands = doc["commands"].as_object().unwrap();
        for name in [
            "list",
            "status",
            "stats",
            "watch",
            "config get",
            "config set",
            "config validate",
            "export",
            "import",
            "import-legacy",
        ] {
            assert!(commands.contains_key(name), "missing command: {name}");
        }
        let defs = doc["$defs"].as_object().unwrap();
        assert!(defs.contains_key("ResponseData"));
        assert!(defs.contains_key("Entry"));
        assert!(defs.contains_key("Config"));

        // Every `$ref` in `commands` (and transitively inside `$defs`)
        // resolves to a real `$defs` entry, so the document is internally
        // consistent and not just individually-valid fragments.
        fn walk(value: &Value, defs: &Map<String, Value>) {
            match value {
                Value::Object(map) => {
                    if let Some(Value::String(r)) = map.get("$ref") {
                        let name = r
                            .strip_prefix("#/$defs/")
                            .unwrap_or_else(|| panic!("unexpected $ref shape: {r}"));
                        assert!(defs.contains_key(name), "dangling $ref: {r}");
                    }
                    for v in map.values() {
                        walk(v, defs);
                    }
                }
                Value::Array(items) => {
                    for v in items {
                        walk(v, defs);
                    }
                }
                _ => {}
            }
        }
        walk(&doc["commands"].clone(), defs);
        walk(&doc["$defs"].clone(), defs);
    }

    #[test]
    fn schema_version_is_a_const_in_the_document() {
        let doc = document();
        assert_eq!(doc["schema_version"], JSON_SCHEMA_VERSION);
    }
}
