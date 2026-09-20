#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const extensionDir = path.join(root, "gnome-extension");
const metadataPath = path.join(extensionDir, "metadata.json");
const extensionPath = path.join(extensionDir, "extension.js");
const metadata = JSON.parse(fs.readFileSync(metadataPath, "utf8"));
const source = fs.readFileSync(extensionPath, "utf8");
const prefsPath = path.join(extensionDir, "prefs.js");
const prefs = fs.readFileSync(prefsPath, "utf8");

const required = ["uuid", "name", "description", "shell-version"];
for (const field of required) {
  if (!(field in metadata)) throw new Error(`metadata missing ${field}`);
}
if (metadata.uuid !== "panora@ygkali.github.io") throw new Error("unexpected extension UUID");
if (!Array.isArray(metadata["shell-version"]) || metadata["shell-version"].length === 0) {
  throw new Error("shell-version must be a non-empty array");
}
if (/\b(eval|Function|fetch|XMLHttpRequest|WebSocket)\s*\(/.test(source)) {
  throw new Error("extension contains a dynamic/network execution primitive");
}
if (/https?:\/\//i.test(source)) throw new Error("extension contains an unexpected network URL");
if (!source.includes("Gio.BusType.SESSION") || !source.includes("io.github.ygkali.Panora.GnomeBridge1")) {
  throw new Error("extension must use the expected session D-Bus boundary");
}
// The helper service the daemon calls back into must stay tiny and fixed.
const exportedMethods = [...source.matchAll(/<method name="([A-Za-z]+)"/g)].map((m) => m[1]).sort();
if (exportedMethods.join(",") !== "Paste,SetClipboard") {
  throw new Error(`unexpected helper methods exported: ${exportedMethods.join(", ")}`);
}
if (!source.includes("io.github.ygkali.Panora.GnomeShell1") || !source.includes("Gio.DBusExportedObject")) {
  throw new Error("extension must export the io.github.ygkali.Panora.GnomeShell1 helper");
}
if (/GLib\.spawn_(async|sync|command_line)/.test(source.replace(/\[POPUP_BINARY\]/g, ""))
  && !/GLib\.spawn_async\(null, \[POPUP_BINARY\]/.test(source)) {
  throw new Error("extension may only spawn the fixed popup binary");
}
// prefs.js runs outside the Shell but ships in the same package, so the
// same primitives are out of bounds there.
if (/\b(eval|Function|fetch|XMLHttpRequest|WebSocket)\s*\(/.test(prefs)) {
  throw new Error("prefs.js contains a dynamic/network execution primitive");
}
if (/https?:\/\//i.test(prefs)) throw new Error("prefs.js contains an unexpected network URL");
if (/GLib\.spawn|Gio\.Subprocess/.test(prefs)) throw new Error("prefs.js must not start processes");
if (metadata["settings-schema"] !== "org.gnome.shell.extensions.panora") {
  throw new Error("metadata must declare the settings schema prefs.js opens");
}
console.log(`extension-security: PASS (${metadata.uuid}, ${metadata["shell-version"].join(", ")})`);
