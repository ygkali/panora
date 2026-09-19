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
//
// Verified against GNOME Shell / Mutter 46 (Zorin OS 18, Ubuntu 24.04) and
// kept within the API surface that is unchanged from 45 to 48.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const BRIDGE_NAME = 'io.github.ygkali.Panora.GnomeBridge1';
const BRIDGE_PATH = '/io/github/ygkali/Panora/GnomeBridge1';
const HELPER_NAME = 'io.github.ygkali.Panora.GnomeShell1';
const HELPER_PATH = '/io/github/ygkali/Panora/GnomeShell1';
const KEYBINDING = 'toggle-popup';
const RESTORE_KEY = 'restore-message-tray';
const APP_BUS_NAME = 'io.github.ygkali.Panora';
const APP_OBJECT_PATH = '/io/github/ygkali/Panora';
const APP_DESKTOP_ID = 'io.github.ygkali.Panora.desktop';
// Where the Debian package installs the popup (fallback when the desktop
// entry is not visible to the Shell, e.g. source installs).
const POPUP_BINARY = '/usr/bin/panora';

// GNOME's own binding for the notification list. Its default value is
// ['<Super>v', '<Super>m'] (data/org.gnome.shell.gschema.xml.in), so it
// collides with the Panora shortcut; see _claimShortcut().
const SHELL_KEYBINDINGS_SCHEMA = 'org.gnome.shell.keybindings';
const MESSAGE_TRAY_KEY = 'toggle-message-tray';

// Mirrors panora-core's privacy markers. Content a password manager flagged
// as secret is dropped here, before any payload is read.
const SECRET_MARKERS = ['passwordmanagerhint', 'concealedtype', 'clipboard viewer ignore'];

// One clipboard change is pushed as up to a handful of payloads
// (PushMany). If an image is offered only the best image is sent; otherwise
// the best plain-text flavour is sent together with text/html and the file
// list flavours when present, so browser copies stay pasteable in terminals
// and file copies keep their URIs. The first entry of the list is the
// primary payload (the one the legacy single-payload Push() carries).
const IMAGE_MIMES = ['image/png', 'image/jpeg', 'image/webp'];
const TEXT_MIMES = ['text/plain;charset=utf-8', 'text/plain', 'UTF8_STRING'];
const EXTRA_MIMES = ['text/html', 'text/uri-list', 'x-special/gnome-copied-files'];

// Mirrors panora-core's default HistoryConfig::max_mime_bytes.
const MAX_PAYLOAD_BYTES = 10 * 1024 * 1024;

// Coalesce the burst of owner-changed signals a single copy can produce.
const DEBOUNCE_MS = 120;
const CALL_TIMEOUT_MS = 2000;
// A cold GTK start (D-Bus activation compiling CSS, loading icons) can take
// seconds on a slow disk; the Activate reply only arrives once the
// application has started up.
const ACTIVATE_TIMEOUT_MS = 25000;
// After SetClipboard() the Shell's own owner-changed must not be pushed
// back to the daemon as a new entry.
const SUPPRESS_ECHO_US = 1500000;

// Paste(): how long to wait for keyboard focus to leave the Panora popup
// before giving up, and how often to look.
const PASTE_FOCUS_TIMEOUT_MS = 700;
const PASTE_POLL_MS = 25;

// Linux evdev key codes (input-event-codes.h). Clutter's
// notify_key() takes evdev codes on both backends: the native backend
// injects them as-is and the X11 backend adds the 8 offset for XTest. Using
// key codes instead of keyvals keeps Ctrl+V working on layouts that have no
// Latin "v" (Cyrillic, Greek, ...), where notify_keyval() would log
// "No keycode found for keyval" and send nothing.
const KEY_LEFTCTRL = 29;
const KEY_V = 47;

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

function normalizeAccelerator(accelerator) {
    return accelerator.replace(/\s+/g, '').toLowerCase();
}

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
        this._pendingPastes = new Set();
        this._suppressUntil = 0;
        this._shellKeybindings = null;
        this._shortcutChangedId = 0;

        this._settings = this.getSettings();
        Main.wm.addKeybinding(
            KEYBINDING,
            this._settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this._openPopup()
        );
        this._claimShortcut();
        this._shortcutChangedId = this._settings.connect(
            `changed::${KEYBINDING}`,
            () => {
                this._releaseShortcut();
                this._claimShortcut();
            }
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

        // MetaSelection::owner-changed (selection-type: uint, source) fires
        // for every clipboard owner change, including ones made by X11
        // clients through Xwayland and by the Shell itself.
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

        if (this._shortcutChangedId) {
            this._settings.disconnect(this._shortcutChangedId);
            this._shortcutChangedId = 0;
        }
        this._releaseShortcut();
        this._shellKeybindings = null;

        if (this._debounceId) {
            GLib.Source.remove(this._debounceId);
            this._debounceId = 0;
        }
        for (const pending of this._pendingPastes) {
            if (pending.sourceId)
                GLib.Source.remove(pending.sourceId);
            pending.invocation.return_dbus_error(
                `${HELPER_NAME}.Error.Disabled`,
                'the Panora extension was disabled'
            );
        }
        this._pendingPastes.clear();
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

    // --------------------------------------------------------- shortcut

    // Mutter indexes key bindings in a hash table keyed by (keycode, mask)
    // and keeps ONE binding per combo: the last one indexed wins
    // (src/core/keybindings.c, index_binding → g_hash_table_replace), and
    // the whole index is rebuilt in hash-table order whenever any binding
    // or the keyboard layout changes. So with GNOME's default
    // toggle-message-tray = ['<Super>v', '<Super>m'] in place, Super+V would
    // open Panora or the notification list depending on which binding
    // happened to be indexed last. Nothing in the Shell resolves the
    // conflict, so it is resolved here: the colliding entries are removed
    // from toggle-message-tray (Super+M keeps working), the original list is
    // remembered in this extension's settings and put back on disable().
    _claimShortcut() {
        try {
            if (!this._shellKeybindings)
                this._shellKeybindings = new Gio.Settings({schema_id: SHELL_KEYBINDINGS_SCHEMA});
            const ours = this._settings.get_strv(KEYBINDING).map(normalizeAccelerator);
            if (ours.length === 0)
                return;
            const tray = this._shellKeybindings.get_strv(MESSAGE_TRAY_KEY);
            const kept = tray.filter(accel => !ours.includes(normalizeAccelerator(accel)));
            if (kept.length === tray.length)
                return;
            // Never overwrite a saved value: if the previous session ended
            // without disable() running, it still holds the user's original.
            if (this._settings.get_strv(RESTORE_KEY).length === 0)
                this._settings.set_strv(RESTORE_KEY, tray);
            this._shellKeybindings.set_strv(MESSAGE_TRAY_KEY, kept);
        } catch (error) {
            logError(error, 'Panora: could not take over the shortcut');
        }
    }

    _releaseShortcut() {
        try {
            const saved = this._settings.get_strv(RESTORE_KEY);
            if (saved.length === 0)
                return;
            if (!this._shellKeybindings)
                this._shellKeybindings = new Gio.Settings({schema_id: SHELL_KEYBINDINGS_SCHEMA});
            const current = this._shellKeybindings.get_strv(MESSAGE_TRAY_KEY);
            // Only restore what this extension wrote: the current value must
            // still be a subset of the saved one. If the user gave the
            // notification list a new shortcut meanwhile, their value stays.
            if (current.every(accel => saved.includes(accel)))
                this._shellKeybindings.set_strv(MESSAGE_TRAY_KEY, saved);
            this._settings.set_strv(RESTORE_KEY, []);
        } catch (error) {
            logError(error, 'Panora: could not restore the notification shortcut');
        }
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
                new GLib.Variant('(a{sv})', [this._activationPlatformData()]),
                null,
                Gio.DBusCallFlags.NONE,
                ACTIVATE_TIMEOUT_MS,
                null,
                (connection, result) => {
                    try {
                        connection.call_finish(result);
                    } catch (error) {
                        // Only spawn when nothing owns the name and D-Bus
                        // could not start it (no service file, e.g. a source
                        // checkout). A timeout or any other failure means an
                        // instance is (being) started: spawning a second one
                        // would just toggle the fresh window closed again.
                        if (this._isNoOwnerError(error))
                            this._spawnPopup();
                        else
                            logError(error, 'Panora: popup activation failed');
                    }
                }
            );
            return;
        }
        this._spawnPopup();
    }

    _isNoOwnerError(error) {
        if (error.matches?.(Gio.DBusError, Gio.DBusError.SERVICE_UNKNOWN) ||
            error.matches?.(Gio.DBusError, Gio.DBusError.NAME_HAS_NO_OWNER))
            return true;
        if (!Gio.DBusError.is_remote_error(error))
            return false;
        const name = Gio.DBusError.get_remote_error(error);
        return name === 'org.freedesktop.DBus.Error.ServiceUnknown' ||
            name === 'org.freedesktop.DBus.Error.NameHasNoOwner';
    }

    // Mutter grants focus to a presented window only with a valid activation
    // token (xdg-activation on Wayland, startup notification on X11);
    // without one the popup can end up behind the current window with a
    // "Panora is ready" notification. The Shell's launch context mints a
    // token the compositor recognises, and GtkApplication picks it up from
    // the 'activation-token' / 'desktop-startup-id' platform data.
    _activationPlatformData() {
        try {
            const appInfo = Gio.DesktopAppInfo.new(APP_DESKTOP_ID);
            if (!appInfo)
                return {};
            const context = global.create_app_launch_context(global.get_current_time(), -1);
            const token = context.get_startup_notify_id(appInfo, []);
            if (!token)
                return {};
            return {
                'activation-token': new GLib.Variant('s', token),
                'desktop-startup-id': new GLib.Variant('s', token),
            };
        } catch (_error) {
            return {};
        }
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
    // St.Clipboard.set_content(type, mimetype, GBytes) creates a
    // MetaSelectionSourceMemory that offers exactly that one MIME type; for
    // 'image/png' that is what GTK, Qt, LibreOffice and browsers request
    // when pasting an image. set_text() is the same call with
    // 'text/plain;charset=utf-8', which Xwayland maps to UTF8_STRING for
    // X11 clients.
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
        // The Shell's own owner-changed follows; the daemon already holds
        // this content (and has its own echo detection), so captures are
        // muted briefly instead of pushing the recall back as a new entry.
        this._suppressUntil = GLib.get_monotonic_time() + SUPPRESS_ECHO_US;
    }

    // Paste(): Ctrl+V into the focused window through a virtual keyboard.
    // Implemented as the *Async variant so the reply can wait for keyboard
    // focus to leave the Panora popup: the popup hides itself and the daemon
    // calls this ~200 ms later, but on a loaded system the unmap may not
    // have been processed yet, and Ctrl+V would then land in the popup.
    PasteAsync(_params, invocation) {
        const pending = {invocation, sourceId: 0};
        this._pendingPastes.add(pending);
        const deadline = GLib.get_monotonic_time() + PASTE_FOCUS_TIMEOUT_MS * 1000;

        const attempt = () => {
            pending.sourceId = 0;
            if (this._popupHasFocus()) {
                if (GLib.get_monotonic_time() < deadline) {
                    pending.sourceId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, PASTE_POLL_MS, attempt);
                    return GLib.SOURCE_REMOVE;
                }
                this._pendingPastes.delete(pending);
                invocation.return_dbus_error(
                    `${HELPER_NAME}.Error.Focus`,
                    'the Panora popup still has keyboard focus'
                );
                return GLib.SOURCE_REMOVE;
            }
            this._pendingPastes.delete(pending);
            try {
                this._sendCtrlV();
                invocation.return_value(null);
            } catch (error) {
                invocation.return_dbus_error(`${HELPER_NAME}.Error.Paste`, String(error.message ?? error));
            }
            return GLib.SOURCE_REMOVE;
        };
        attempt();
    }

    _popupHasFocus() {
        const window = global.display.focus_window;
        if (!window)
            return false;
        // Wayland app_id and the X11 WM_CLASS class both equal the
        // GApplication id for GTK4 windows.
        if (window.get_wm_class() === APP_BUS_NAME)
            return true;
        const app = Shell.WindowTracker.get_default().get_window_app(window);
        return app !== null && app.get_id() === APP_DESKTOP_ID;
    }

    _sendCtrlV() {
        if (!this._virtualKeyboard) {
            const seat = global.backend.get_default_seat();
            this._virtualKeyboard = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        }
        const keyboard = this._virtualKeyboard;
        // Events are queued in order on the seat; no extra delay is needed
        // between press and release. Wayland clients get them from the
        // compositor seat, X11 clients through Xwayland (or XTest on the
        // X11 backend), so the same code covers both window types.
        const now = () => GLib.get_monotonic_time();
        keyboard.notify_key(now(), KEY_LEFTCTRL, Clutter.KeyState.PRESSED);
        keyboard.notify_key(now(), KEY_V, Clutter.KeyState.PRESSED);
        keyboard.notify_key(now(), KEY_V, Clutter.KeyState.RELEASED);
        keyboard.notify_key(now(), KEY_LEFTCTRL, Clutter.KeyState.RELEASED);
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
        if (GLib.get_monotonic_time() < this._suppressUntil)
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
        // GList<utf8> (transfer full) → JS string array.
        const mimes = clipboard.get_mimetypes(St.ClipboardType.CLIPBOARD);
        if (!mimes || mimes.length === 0)
            return;

        const lowered = mimes.map(mime => mime.toLowerCase());
        const secret = lowered.some(
            mime => SECRET_MARKERS.some(marker => mime.includes(marker))
        );
        if (secret)
            return;

        const wanted = [];
        const pick = candidates => {
            for (const candidate of candidates) {
                const index = lowered.indexOf(candidate.toLowerCase());
                if (index >= 0)
                    return mimes[index];
            }
            return null;
        };
        const image = pick(IMAGE_MIMES);
        if (image !== null) {
            wanted.push(image);
        } else {
            const text = pick(TEXT_MIMES);
            if (text !== null)
                wanted.push(text);
            for (const extra of EXTRA_MIMES) {
                const found = pick([extra]);
                if (found !== null)
                    wanted.push(found);
            }
        }
        if (wanted.length === 0)
            return;

        // get_content() transfers the selection into a memory stream and
        // hands the callback a GLib.Bytes (null on failure) for every MIME
        // type, text or binary; get_data() views it as a Uint8Array. The
        // formats are read one after another and pushed together.
        const payloads = [];
        const readNext = () => {
            if (wanted.length === 0) {
                if (payloads.length > 0)
                    this._pushMany(mimes, payloads);
                return;
            }
            const mime = wanted.shift();
            clipboard.get_content(St.ClipboardType.CLIPBOARD, mime, (_source, bytes) => {
                const data = bytes ? bytes.get_data() : null;
                // Matches the daemon's default max_mime_bytes. Dropping
                // oversized payloads here avoids pushing megabytes across the
                // session bus only for the daemon to discard them.
                if (data && data.length > 0 && data.length <= MAX_PAYLOAD_BYTES)
                    payloads.push([mime, data]);
                readNext();
            });
        };
        readNext();
    }

    // PushMany(mimes: as, payloads: a(say), source_app: s). Falls back to the
    // single-payload Push() for daemons that predate it.
    _pushMany(mimes, payloads) {
        if (!this._bus)
            return;
        const variant = new GLib.Variant('(asa(say)s)', [mimes, payloads, this._sourceApp()]);
        this._bus.call(
            BRIDGE_NAME,
            BRIDGE_PATH,
            BRIDGE_NAME,
            'PushMany',
            variant,
            null,
            Gio.DBusCallFlags.NO_AUTO_START,
            CALL_TIMEOUT_MS,
            null,
            (connection, result) => {
                try {
                    connection.call_finish(result);
                } catch (error) {
                    if (this._isUnknownMethodError(error)) {
                        const [mime, data] = payloads[0];
                        this._push(mimes, mime, data);
                    }
                    // Otherwise the daemon is down, paused or busy. Dropping
                    // the event is the correct outcome; the next copy will be
                    // captured.
                }
            }
        );
    }

    _isUnknownMethodError(error) {
        if (error.matches?.(Gio.DBusError, Gio.DBusError.UNKNOWN_METHOD))
            return true;
        return Gio.DBusError.is_remote_error(error) &&
            Gio.DBusError.get_remote_error(error) === 'org.freedesktop.DBus.Error.UnknownMethod';
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
