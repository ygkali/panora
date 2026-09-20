// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Preferences for the Panora GNOME Shell bridge.
//
// Only the two settings that belong to the Shell side live here: the
// shortcut that opens the popup, and whether the popup is moved to the
// pointer. Everything else -- history limits, the privacy rules, the theme
// -- is the daemon's and is edited in the popup's own settings dialog,
// because those settings live in config.toml and not in dconf.
//
// Runs in the preferences process, not in the Shell: GTK and libadwaita are
// available here and `global` is not.

import Adw from 'gi://Adw';
import Gdk from 'gi://Gdk';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';
import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

const KEYBINDING = 'toggle-popup';
const POINTER_KEY = 'move-to-pointer';

// Keys that mean "nothing was pressed yet" while a modifier is held down.
const MODIFIER_KEYVALS = [
    Gdk.KEY_Alt_L, Gdk.KEY_Alt_R,
    Gdk.KEY_Control_L, Gdk.KEY_Control_R,
    Gdk.KEY_Shift_L, Gdk.KEY_Shift_R,
    Gdk.KEY_Super_L, Gdk.KEY_Super_R,
    Gdk.KEY_Meta_L, Gdk.KEY_Meta_R,
    Gdk.KEY_Hyper_L, Gdk.KEY_Hyper_R,
    Gdk.KEY_ISO_Level3_Shift, Gdk.KEY_ISO_Level5_Shift,
];

// A shortcut with no modifier would swallow the key everywhere in the
// session, so only the function keys are allowed on their own.
function isAcceptable(keyval, mask) {
    if (MODIFIER_KEYVALS.includes(keyval)) {
        return false;
    }
    if (mask !== 0) {
        return Gtk.accelerator_valid(keyval, mask);
    }
    return keyval >= Gdk.KEY_F1 && keyval <= Gdk.KEY_F12;
}

export default class PanoraPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings();

        const page = new Adw.PreferencesPage({
            title: 'Panora',
            icon_name: 'edit-paste-symbolic',
        });

        const group = new Adw.PreferencesGroup({
            title: 'Shell integration',
            description:
                'The history, the privacy rules and the appearance of the ' +
                'popup are set in the popup itself (Ctrl+comma), not here.',
        });

        group.add(this._shortcutRow(window, settings));
        group.add(this._pointerRow(settings));

        page.add(group);
        window.add(page);
    }

    // An activatable row showing the current accelerator; activating it
    // opens a dialog that records the next combination pressed.
    _shortcutRow(window, settings) {
        const label = new Gtk.ShortcutLabel({
            valign: Gtk.Align.CENTER,
            disabled_text: 'Disabled',
            accelerator: settings.get_strv(KEYBINDING)[0] ?? '',
        });
        settings.connect(`changed::${KEYBINDING}`, () => {
            label.set_accelerator(settings.get_strv(KEYBINDING)[0] ?? '');
        });

        const row = new Adw.ActionRow({
            title: 'Open the clipboard history',
            subtitle:
                'GNOME binds Super+V to the notification list; the ' +
                'extension takes it over and gives it back when disabled.',
            activatable: true,
        });
        row.add_suffix(label);

        const reset = new Gtk.Button({
            icon_name: 'edit-clear-symbolic',
            valign: Gtk.Align.CENTER,
            tooltip_text: 'Back to Super+V',
            css_classes: ['flat'],
        });
        reset.connect('clicked', () => settings.reset(KEYBINDING));
        row.add_suffix(reset);

        row.connect('activated', () => this._capture(window, settings));
        return row;
    }

    _pointerRow(settings) {
        const row = new Adw.SwitchRow({
            title: 'Open next to the pointer',
            subtitle:
                'Move the popup to the mouse pointer once it appears, the ' +
                'way the Windows clipboard flyout does.',
        });
        settings.bind(POINTER_KEY, row, 'active', Gio.SettingsBindFlags.DEFAULT);
        return row;
    }

    // Modal dialog that listens for one key combination and writes it.
    _capture(window, settings) {
        const dialog = new Adw.Window({
            transient_for: window,
            modal: true,
            default_width: 420,
            default_height: 220,
            title: 'Set the shortcut',
        });

        const status = new Adw.StatusPage({
            title: 'Press the new shortcut',
            description: 'Escape cancels, Backspace disables the shortcut.',
            icon_name: 'preferences-desktop-keyboard-shortcuts-symbolic',
        });

        const view = new Adw.ToolbarView({content: status});
        view.add_top_bar(new Adw.HeaderBar({show_end_title_buttons: false}));
        dialog.set_content(view);

        const controller = new Gtk.EventControllerKey();
        controller.connect('key-pressed', (_controller, keyval, keycode, state) => {
            const mask = state & Gtk.accelerator_get_default_mod_mask() &
                ~Gdk.ModifierType.LOCK_MASK;

            if (keyval === Gdk.KEY_Escape && mask === 0) {
                dialog.close();
                return Gdk.EVENT_STOP;
            }
            if (keyval === Gdk.KEY_BackSpace && mask === 0) {
                settings.set_strv(KEYBINDING, []);
                dialog.close();
                return Gdk.EVENT_STOP;
            }
            if (!isAcceptable(keyval, mask)) {
                status.set_description(
                    'That one cannot be a shortcut on its own. Hold a ' +
                    'modifier, or use a function key.');
                return Gdk.EVENT_STOP;
            }

            const accelerator =
                Gtk.accelerator_name_with_keycode(null, keyval, keycode, mask);
            settings.set_strv(KEYBINDING, [accelerator]);
            dialog.close();
            return Gdk.EVENT_STOP;
        });
        dialog.add_controller(controller);

        dialog.present();
    }
}
