#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(new URL("..", import.meta.url).pathname);
const extensionDir = path.join(root, "gnome-extension");
const metadataPath = path.join(extensionDir, "metadata.json");
const extensionPath = path.join(extensionDir, "extension.js");
const metadata = JSON.parse(fs.readFileSync(metadataPath, "utf8"));
const source = fs.readFileSync(extensionPath, "utf8");

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
console.log(`extension-security: PASS (${metadata.uuid}, ${metadata["shell-version"].join(", ")})`);
