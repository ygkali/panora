// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Panora GNOME Shell bridge.
//
// Two jobs, both local:
//   1. Bind Super+V to the Panora popup.
//   2. On Wayland only, forward clipboard changes to the daemon over the
//      session bus, because Mutter denies non-focused clients clipboard reads.
// On X11 the daemon reads the clipboard itself, so the forwarder stays off.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const BRIDGE_NAME = 'io.panora.GnomeBridge1';
const BRIDGE_PATH = '/io/panora/GnomeBridge1';
const KEYBINDING = 'toggle-popup';
// Where the Debian package installs the popup.
const POPUP_BINARY = '/usr/bin/panora';

// Mirrors panora-core's privacy markers. Content a password manager flagged
// as secret is dropped here, before any payload is read.
const SECRET_MARKERS = ['passwordmanagerhint', 'concealedtype'];

// The bridge forwards exactly one payload per event; this is the preference
// order used to pick it.
const MIME_PRIORITY = [
    'image/png',
    'image/jpeg',
    'image/webp',
    'text/uri-list',
    'text/html',
    'text/plain;charset=utf-8',
    'text/plain',
    'UTF8_STRING',
];

// Mirrors panora-core's default HistoryConfig::max_mime_bytes.
const MAX_PAYLOAD_BYTES = 10 * 1024 * 1024;

// Coalesce the burst of owner-changed signals a single copy can produce.
const DEBOUNCE_MS = 120;
const CALL_TIMEOUT_MS = 2000;

export default class PanoraExtension extends Extension {
    enable() {
        this._bus = null;
        this._selection = null;
        this._ownerChangedId = 0;
        this._debounceId = 0;

        this._settings = this.getSettings();
        Main.wm.addKeybinding(
            KEYBINDING,
            this._settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this._openPopup()
        );

        if (!Meta.is_wayland_compositor())
            return;

        try {
            this._bus = Gio.bus_get_sync(Gio.BusType.SESSION, null);
        } catch (error) {
            logError(error, 'Panora: session bus unavailable');
            return;
        }

        this._selection = global.display.get_selection();
        this._ownerChangedId = this._selection.connect(
            'owner-changed',
            (_selection, type) => {
                if (type === Meta.SelectionType.SELECTION_CLIPBOARD)
                    this._scheduleCapture();
            }
        );
    }

    disable() {
        Main.wm.removeKeybinding(KEYBINDING);

        if (this._debounceId) {
            GLib.Source.remove(this._debounceId);
            this._debounceId = 0;
        }
        if (this._selection && this._ownerChangedId) {
            this._selection.disconnect(this._ownerChangedId);
            this._ownerChangedId = 0;
        }
        this._selection = null;
        this._bus = null;
        this._settings = null;
    }

    _openPopup() {
        try {
            // Absolute path on purpose: this runs inside the gnome-shell
            // process, so resolving through $PATH would let anything earlier
            // on the session PATH take over the shortcut. Source installs that
            // land outside /usr/bin should adjust POPUP_BINARY.
            GLib.spawn_async(null, [POPUP_BINARY], null, GLib.SpawnFlags.DEFAULT, null);
        } catch (error) {
            logError(error, 'Panora: could not launch the popup');
        }
    }

    _scheduleCapture() {
        if (this._debounceId)
            GLib.Source.remove(this._debounceId);

        this._debounceId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, DEBOUNCE_MS, () => {
            this._debounceId = 0;
            this._capture();
            return GLib.SOURCE_REMOVE;
        });
    }

    _capture() {
        const clipboard = St.Clipboard.get_default();
        const mimes = clipboard.get_mimetypes(St.ClipboardType.CLIPBOARD);
        if (!mimes || mimes.length === 0)
            return;

        const lowered = mimes.map(mime => mime.toLowerCase());
        const secret = lowered.some(
            mime => SECRET_MARKERS.some(marker => mime.includes(marker))
        );
        if (secret)
            return;

        let chosen = null;
        for (const candidate of MIME_PRIORITY) {
            const index = lowered.indexOf(candidate.toLowerCase());
            if (index >= 0) {
                chosen = mimes[index];
                break;
            }
        }
        if (chosen === null)
            return;

        clipboard.get_content(St.ClipboardType.CLIPBOARD, chosen, (_source, bytes) => {
            if (!bytes)
                return;
            const data = bytes.get_data();
            if (!data || data.length === 0)
                return;
            // Matches the daemon's default max_mime_bytes. Dropping oversized
            // payloads here avoids pushing megabytes across the session bus
            // only for the daemon to discard them.
            if (data.length > MAX_PAYLOAD_BYTES)
                return;
            this._push(mimes, chosen, data);
        });
    }

    _push(mimes, mime, data) {
        if (!this._bus)
            return;

        // Push(mimes: as, mime: s, bytes: ay, source_app: s)
        const payload = new GLib.Variant('(assays)', [mimes, mime, data, this._sourceApp()]);
        this._bus.call(
            BRIDGE_NAME,
            BRIDGE_PATH,
            BRIDGE_NAME,
            'Push',
            payload,
            null,
            Gio.DBusCallFlags.NO_AUTO_START,
            CALL_TIMEOUT_MS,
            null,
            (connection, result) => {
                try {
                    connection.call_finish(result);
                } catch (_error) {
                    // The daemon is down, paused or busy. Dropping the event is
                    // the correct outcome; the next copy will be captured.
                }
            }
        );
    }

    _sourceApp() {
        const window = global.display.focus_window;
        if (!window)
            return '';
        const app = Shell.WindowTracker.get_default().get_window_app(window);
        return app ? app.get_id() : '';
    }
}
