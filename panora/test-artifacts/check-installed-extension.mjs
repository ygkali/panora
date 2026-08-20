#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

const root = process.env.PANORA_EXTENSION_DIR || "/usr/share/gnome-shell/extensions/panora@panora-clipboard.org";
const metadata = JSON.parse(fs.readFileSync(path.join(root, "metadata.json"), "utf8"));
const source = fs.readFileSync(path.join(root, "extension.js"), "utf8");
const required = ["uuid", "name", "description", "shell-version"];
for (const field of required) {
  if (!(field in metadata)) throw new Error(`metadata missing ${field}`);
}
if (metadata.uuid !== "panora@panora-clipboard.org") throw new Error("unexpected extension UUID");
if (!Array.isArray(metadata["shell-version"]) || metadata["shell-version"].length === 0) {
  throw new Error("shell-version must be a non-empty array");
}
if (/\b(eval|Function|fetch|XMLHttpRequest|WebSocket)\s*\(/.test(source)) {
  throw new Error("extension contains a dynamic/network execution primitive");
}
if (/https?:\/\//i.test(source)) throw new Error("extension contains an unexpected network URL");
if (!source.includes("Gio.BusType.SESSION") || !source.includes("io.panora.GnomeBridge1")) {
  throw new Error("extension must use the expected session D-Bus boundary");
}
console.log(`installed-extension-security: PASS (${metadata.uuid}, ${metadata["shell-version"].join(", ")})`);
