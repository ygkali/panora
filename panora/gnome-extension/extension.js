// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Panora GNOME Shell bridge.
//
// Three jobs, all local:
//   1. Bind Super+V to the Panora popup (activates the running instance,
//      which toggles it, or launches it through the .desktop entry).
//   2. On Wayland without a data-control protocol (GNOME < 48), forward
//      clipboard changes to the daemon over the session bus, because Mutter
//      denies non-focused clients clipboard reads. On newer GNOME the daemon
//      captures natively and ignores these pushes.
//   3. Export a tiny helper service the daemon calls to set the clipboard
//      (recall on GNOME < 48) and to synthesize Ctrl+V (instant paste).

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const BRIDGE_NAME = 'io.panora.GnomeBridge1';
const BRIDGE_PATH = '/io/panora/GnomeBridge1';
const HELPER_NAME = 'io.panora.GnomeShell1';
const HELPER_PATH = '/io/panora/GnomeShell1';
const KEYBINDING = 'toggle-popup';
const APP_BUS_NAME = 'io.panora.Panora';
const APP_OBJECT_PATH = '/io/panora/Panora';
const APP_DESKTOP_ID = 'io.panora.Panora.desktop';
// Where the Debian package installs the popup (fallback when the desktop
// entry is not visible to the Shell, e.g. source installs).
const POPUP_BINARY = '/usr/bin/panora';

// Mirrors panora-core's privacy markers. Content a password manager flagged
// as secret is dropped here, before any payload is read.
const SECRET_MARKERS = ['passwordmanagerhint', 'concealedtype', 'clipboard viewer ignore'];

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

const HELPER_IFACE = `
<node>
  <interface name="${HELPER_NAME}">
    <method name="SetClipboard">
      <arg type="s" direction="in" name="mime"/>
      <arg type="ay" direction="in" name="bytes"/>
    </method>
    <method name="Paste"/>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;

export default class PanoraExtension extends Extension {
    enable() {
        this._bus = null;
        this._selection = null;
        this._ownerChangedId = 0;
        this._debounceId = 0;
        this._helper = null;
        this._nameId = 0;
        this._watchId = 0;
        this._needsBridge = false;
        this._virtualKeyboard = null;

        this._settings = this.getSettings();
        Main.wm.addKeybinding(
            KEYBINDING,
            this._settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this._openPopup()
        );

        try {
            this._bus = Gio.bus_get_sync(Gio.BusType.SESSION, null);
        } catch (error) {
            logError(error, 'Panora: session bus unavailable');
            return;
        }

        this._exportHelper();

        if (!Meta.is_wayland_compositor())
            return;

        // Only forward clipboard changes while the daemon says it depends on
        // them (GNOME without a data-control protocol). On newer GNOME the
        // daemon captures natively and pushes would be discarded, so the
        // clipboard is not read at all.
        this._needsBridge = false;
        this._watchId = Gio.bus_watch_name_on_connection(
            this._bus,
            BRIDGE_NAME,
            Gio.BusNameWatcherFlags.NONE,
            () => this._refreshNeedsBridge(),
            () => { this._needsBridge = false; }
        );

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
        if (this._watchId) {
            Gio.bus_unwatch_name(this._watchId);
            this._watchId = 0;
        }
        if (this._nameId) {
            Gio.bus_unown_name(this._nameId);
            this._nameId = 0;
        }
        if (this._helper) {
            this._helper.unexport();
            this._helper = null;
        }
        this._virtualKeyboard = null;
        this._selection = null;
        this._bus = null;
        this._settings = null;
    }

    // ------------------------------------------------------------ popup

    _openPopup() {
        // org.freedesktop.Application.Activate reaches the running instance
        // (which toggles its window) and, through the D-Bus service file,
        // starts the popup outside any sandbox when it is not running.
        // Shell.App.activate() would only focus an existing window.
        if (this._bus) {
            this._bus.call(
                APP_BUS_NAME,
                APP_OBJECT_PATH,
                'org.freedesktop.Application',
                'Activate',
                new GLib.Variant('(a{sv})', [{}]),
                null,
                Gio.DBusCallFlags.NONE,
                CALL_TIMEOUT_MS,
                null,
                (connection, result) => {
                    try {
                        connection.call_finish(result);
                    } catch (_error) {
                        this._spawnPopup();
                    }
                }
            );
            return;
        }
        this._spawnPopup();
    }

    _spawnPopup() {
        const app = Shell.AppSystem.get_default().lookup_app(APP_DESKTOP_ID);
        if (app) {
            try {
                app.activate();
                return;
            } catch (error) {
                logError(error, 'Panora: application activation failed');
            }
        }
        try {
            // Absolute path on purpose: this runs inside the gnome-shell
            // process, so resolving through $PATH would let anything earlier
            // on the session PATH take over the shortcut.
            GLib.spawn_async(null, [POPUP_BINARY], null, GLib.SpawnFlags.DEFAULT, null);
        } catch (error) {
            logError(error, 'Panora: could not launch the popup');
        }
    }

    // ------------------------------------------------------ helper service

    _exportHelper() {
        try {
            this._helper = Gio.DBusExportedObject.wrapJSObject(HELPER_IFACE, this);
            this._helper.export(this._bus, HELPER_PATH);
            this._nameId = Gio.bus_own_name_on_connection(
                this._bus,
                HELPER_NAME,
                Gio.BusNameOwnerFlags.NONE,
                null,
                null
            );
        } catch (error) {
            logError(error, 'Panora: could not export the Shell helper');
            this._helper = null;
        }
    }

    get Version() {
        return 2;
    }

    // SetClipboard(s mime, ay bytes): put one payload on the clipboard.
    SetClipboard(mime, bytes) {
        if (typeof mime !== 'string' || mime.length === 0 || mime.length > 256)
            throw new Error('invalid MIME type');
        const data = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
        if (data.length === 0 || data.length > MAX_PAYLOAD_BYTES)
            throw new Error('payload size out of range');
        const clipboard = St.Clipboard.get_default();
        const lowered = mime.toLowerCase();
        if (lowered.startsWith('text/plain') || lowered === 'utf8_string') {
            clipboard.set_text(St.ClipboardType.CLIPBOARD, new TextDecoder().decode(data));
        } else {
            clipboard.set_content(St.ClipboardType.CLIPBOARD, mime, new GLib.Bytes(data));
        }
    }

    // Paste(): Ctrl+V into the focused window through a virtual keyboard.
    Paste() {
        if (!this._virtualKeyboard) {
            const seat = global.backend.get_default_seat();
            this._virtualKeyboard = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        }
        const keyboard = this._virtualKeyboard;
        const now = () => GLib.get_monotonic_time();
        keyboard.notify_keyval(now(), Clutter.KEY_Control_L, Clutter.KeyState.PRESSED);
        keyboard.notify_keyval(now(), Clutter.KEY_v, Clutter.KeyState.PRESSED);
        keyboard.notify_keyval(now(), Clutter.KEY_v, Clutter.KeyState.RELEASED);
        keyboard.notify_keyval(now(), Clutter.KEY_Control_L, Clutter.KeyState.RELEASED);
    }

    // ------------------------------------------------------------ capture

    _refreshNeedsBridge() {
        if (!this._bus)
            return;
        this._bus.call(
            BRIDGE_NAME,
            BRIDGE_PATH,
            'org.freedesktop.DBus.Properties',
            'Get',
            new GLib.Variant('(ss)', [BRIDGE_NAME, 'NeedsBridge']),
            new GLib.VariantType('(v)'),
            Gio.DBusCallFlags.NO_AUTO_START,
            CALL_TIMEOUT_MS,
            null,
            (connection, result) => {
                try {
                    const [value] = connection.call_finish(result).deep_unpack();
                    this._needsBridge = value.deep_unpack() === true;
                } catch (_error) {
                    // Older daemon without the property: it relies on pushes.
                    this._needsBridge = true;
                }
            }
        );
    }

    _scheduleCapture() {
        if (!this._needsBridge)
            return;
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
