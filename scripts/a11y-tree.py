#!/usr/bin/env python3
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
"""Dump the AT-SPI accessibility tree of the running popup and check it.

Run inside the same session bus as `panora-gui` (see scripts/a11y-check.sh).
Prints one line per accessible object (`indent role: name`) to stdout and
exits 1 when the popup exposes no named controls, which is what a screen
reader would notice first.
"""
import sys
import time

import pyatspi

APP_NAMES = ("panora", "panora-gui", "io.github.ygkali.panora")
WAIT_SECONDS = 30


def find_app():
    desktop = pyatspi.Registry.getDesktop(0)
    for app in desktop:
        if app is None:
            continue
        name = (app.name or "").lower()
        if any(candidate in name for candidate in APP_NAMES):
            return app
    return None


def walk(node, depth, lines, stats):
    try:
        role = node.getRoleName()
        name = node.name or ""
    except Exception:  # noqa: BLE001 - a vanished object is not a failure
        return
    lines.append(f"{'  ' * depth}{role}: {name}")
    if role in ("push button", "toggle button", "menu button") and name:
        stats["named_buttons"] += 1
    if role in ("text", "entry", "password text") or "text" in role:
        stats["text_fields"] += 1
    if name.startswith("TEXT:") or name.startswith("METİN:"):
        stats["labelled_rows"] += 1
    for index in range(node.childCount):
        try:
            child = node.getChildAtIndex(index)
        except Exception:  # noqa: BLE001
            continue
        if child is not None:
            walk(child, depth + 1, lines, stats)


def main():
    deadline = time.time() + WAIT_SECONDS
    app = None
    while time.time() < deadline:
        app = find_app()
        if app is not None:
            break
        time.sleep(0.5)
    if app is None:
        print("a11y: the popup never appeared on the accessibility bus", file=sys.stderr)
        return 1
    # Give the window a moment to fill its list.
    time.sleep(2)
    lines = []
    stats = {"named_buttons": 0, "text_fields": 0, "labelled_rows": 0}
    walk(app, 0, lines, stats)
    print("\n".join(lines))
    print(f"# named buttons: {stats['named_buttons']}, text fields: {stats['text_fields']}, "
          f"labelled rows: {stats['labelled_rows']}")
    problems = []
    if stats["named_buttons"] < 3:
        problems.append("fewer than three named buttons (private mode, menu, clear)")
    if stats["text_fields"] < 1:
        problems.append("no text field (the search box)")
    if stats["labelled_rows"] < 1:
        problems.append("no history row carries a kind-prefixed label")
    for problem in problems:
        print(f"a11y: {problem}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
