// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Preferences dialog. Writes `config.toml` and asks the daemon to reload it
//! so history limits and application exclusions apply immediately.

use crate::util::{apply_theme, call, format_size, kind_label, spawn};
use crate::window::{toast, Ui};
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use panora_core::config::{
    compile_ignore_pattern, data_dir, Config, MAX_IGNORE_PATTERNS, POSITIONS, SENSITIVE_POLICIES,
};
use panora_core::i18n::fill;
use panora_core::ipc::{Request, ResponseData};
use panora_core::model::ContentKind;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const LANGUAGES: [&str; 3] = ["system", "tr", "en"];
const THEMES: [&str; 3] = ["system", "light", "dark"];

/// The kinds a user can switch off, with their `capture_kinds` names.
/// `binary` is not offered: it is whatever failed to classify, and a user
/// who turns off images does not mean to lose those too.
const CAPTURE_KINDS: [(&str, ContentKind); 6] = [
    ("text", ContentKind::Text),
    ("richtext", ContentKind::RichText),
    ("link", ContentKind::Link),
    ("image", ContentKind::Image),
    ("files", ContentKind::FileList),
    ("color", ContentKind::Color),
];

/// Open the preferences dialog.
pub fn show(ui: &Rc<Ui>) {
    open(ui, false);
}

/// Open the preferences dialog on its Devices page.
pub fn show_devices(ui: &Rc<Ui>) {
    open(ui, true);
}

fn open(ui: &Rc<Ui>, devices_first: bool) {
    let s = ui.s;
    let config = ui.app.config.borrow().clone();
    let dialog = adw::PreferencesDialog::builder()
        .title(s.settings_title)
        .build();
    let page = adw::PreferencesPage::builder()
        .title(s.settings_general)
        .icon_name("preferences-system-symbolic")
        .build();

    // --- history -----------------------------------------------------
    let history = adw::PreferencesGroup::builder()
        .title(s.settings_history)
        .build();
    let max_entries = adw::SpinRow::with_range(1.0, 100_000.0, 50.0);
    max_entries.set_title(s.settings_max_entries);
    max_entries.set_subtitle(s.settings_max_entries_sub);
    max_entries.set_value(config.history.max_entries as f64);
    history.add(&max_entries);

    let max_age = adw::SpinRow::with_range(0.0, 36_500.0, 1.0);
    max_age.set_title(s.settings_max_age);
    max_age.set_subtitle(s.settings_max_age_sub);
    max_age.set_value(f64::from(config.history.max_age_days));
    history.add(&max_age);

    let record_primary = adw::SwitchRow::builder()
        .title(s.settings_record_primary)
        .subtitle(s.settings_record_primary_sub)
        .active(config.history.record_primary)
        .build();
    history.add(&record_primary);
    page.add(&history);

    // --- privacy -----------------------------------------------------
    let privacy = adw::PreferencesGroup::builder()
        .title(s.settings_privacy)
        .build();
    let start_private = adw::SwitchRow::builder()
        .title(s.settings_start_private)
        .active(config.privacy.start_private)
        .build();
    privacy.add(&start_private);
    page.add(&privacy);

    let excluded_group = adw::PreferencesGroup::builder()
        .title(s.settings_excluded_apps)
        .description(s.settings_excluded_apps_sub)
        .build();
    let excluded: Rc<RefCell<Vec<String>>> =
        Rc::new(RefCell::new(config.privacy.excluded_apps.clone()));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    rebuild_list(&list, &excluded, s.settings_excluded_remove);

    let add_row = adw::EntryRow::builder()
        .title(s.settings_excluded_add_placeholder)
        .show_apply_button(true)
        .build();
    {
        let excluded = excluded.clone();
        let list = list.clone();
        let remove_label = s.settings_excluded_remove;
        add_row.connect_apply(move |row| {
            let name = row.text().trim().to_ascii_lowercase();
            if name.is_empty() || name.chars().count() > 256 {
                return;
            }
            let mut apps = excluded.borrow_mut();
            if !apps.contains(&name) && apps.len() < 256 {
                apps.push(name);
            }
            drop(apps);
            row.set_text("");
            rebuild_list(&list, &excluded, remove_label);
        });
    }
    excluded_group.add(&add_row);
    excluded_group.add(&list);
    // The list matches on the source application name, which the plain
    // Wayland data-control protocols never expose. Say so here rather than
    // letting the user trust a filter that cannot fire on this session. An
    // unreachable daemon is not evidence of anything, so it warns nothing.
    let source_app_known = match call(&Request::Status) {
        Ok(ResponseData::Status(status)) => status.capabilities.source_app,
        _ => true,
    };
    if !source_app_known {
        let warning = gtk::Label::new(Some(s.settings_excluded_unsupported));
        warning.add_css_class("caption");
        warning.add_css_class("warning");
        warning.set_wrap(true);
        warning.set_xalign(0.0);
        warning.set_margin_top(6);
        excluded_group.add(&warning);
    }
    page.add(&excluded_group);

    // --- content filters ---------------------------------------------
    let filters = adw::PreferencesGroup::builder()
        .title(s.settings_filters)
        .description(s.settings_filters_sub)
        .build();
    let min_text_length = adw::SpinRow::with_range(0.0, 100_000.0, 1.0);
    min_text_length.set_title(s.settings_min_text_length);
    min_text_length.set_subtitle(s.settings_min_text_length_sub);
    min_text_length.set_value(config.privacy.min_text_length as f64);
    filters.add(&min_text_length);

    let ignore_whitespace = adw::SwitchRow::builder()
        .title(s.settings_ignore_whitespace)
        .active(config.privacy.ignore_whitespace_only)
        .build();
    filters.add(&ignore_whitespace);

    let index_full_text = adw::SwitchRow::builder()
        .title(s.settings_index_full_text)
        .subtitle(s.settings_index_full_text_sub)
        .active(config.history.index_full_text)
        .build();
    filters.add(&index_full_text);

    let sensitive_policy = adw::ComboRow::builder()
        .title(s.settings_sensitive_policy)
        .subtitle(s.settings_sensitive_policy_sub)
        .build();
    sensitive_policy.set_model(Some(&gtk::StringList::new(&[
        s.settings_sensitive_mask,
        s.settings_sensitive_drop,
        s.settings_sensitive_store,
    ])));
    sensitive_policy.set_selected(index_of(
        SENSITIVE_POLICIES,
        &config.privacy.sensitive_policy,
    ));
    filters.add(&sensitive_policy);

    let sensitive_ttl = adw::SpinRow::with_range(0.0, 525_600.0, 5.0);
    sensitive_ttl.set_title(s.settings_sensitive_ttl);
    sensitive_ttl.set_subtitle(s.settings_sensitive_ttl_sub);
    sensitive_ttl.set_value(f64::from(config.privacy.sensitive_ttl_minutes));
    filters.add(&sensitive_ttl);

    // One switch per kind; every switch on means "no restriction".
    let kinds_row = adw::ExpanderRow::builder()
        .title(s.settings_capture_kinds)
        .subtitle(s.settings_capture_kinds_sub)
        .build();
    let kind_switches: Vec<(&'static str, adw::SwitchRow)> = CAPTURE_KINDS
        .iter()
        .map(|(name, kind)| {
            let wanted = config.privacy.capture_kinds.is_empty()
                || config.privacy.capture_kinds.iter().any(|k| k == name);
            let row = adw::SwitchRow::builder()
                .title(kind_label(s, *kind))
                .active(wanted)
                .build();
            kinds_row.add_row(&row);
            (*name, row)
        })
        .collect();
    filters.add(&kinds_row);
    page.add(&filters);

    let patterns_group = adw::PreferencesGroup::builder()
        .title(s.settings_ignore_patterns)
        .description(s.settings_ignore_patterns_sub)
        .build();
    let patterns: Rc<RefCell<Vec<String>>> =
        Rc::new(RefCell::new(config.privacy.ignore_patterns.clone()));
    let pattern_list = gtk::ListBox::new();
    pattern_list.add_css_class("boxed-list");
    pattern_list.set_selection_mode(gtk::SelectionMode::None);
    rebuild_list(&pattern_list, &patterns, s.settings_excluded_remove);

    let pattern_row = adw::EntryRow::builder()
        .title(s.settings_ignore_pattern_placeholder)
        .show_apply_button(true)
        .build();
    {
        let patterns = patterns.clone();
        let pattern_list = pattern_list.clone();
        let remove_label = s.settings_excluded_remove;
        let invalid = s.settings_ignore_pattern_invalid;
        pattern_row.connect_apply(move |row| {
            let pattern = row.text().trim().to_string();
            if pattern.is_empty() {
                return;
            }
            // Refuse here what the daemon would refuse on reload, so a typo
            // never turns into a configuration the daemon cannot load.
            if compile_ignore_pattern(&pattern).is_err() {
                row.add_css_class("error");
                row.set_tooltip_text(Some(invalid));
                return;
            }
            row.remove_css_class("error");
            row.set_tooltip_text(None);
            let mut list = patterns.borrow_mut();
            if !list.contains(&pattern) && list.len() < MAX_IGNORE_PATTERNS {
                list.push(pattern);
            }
            drop(list);
            row.set_text("");
            rebuild_list(&pattern_list, &patterns, remove_label);
        });
        pattern_row.connect_changed(|row| {
            row.remove_css_class("error");
            row.set_tooltip_text(None);
        });
    }
    patterns_group.add(&pattern_row);
    patterns_group.add(&pattern_list);
    page.add(&patterns_group);

    let titles_group = adw::PreferencesGroup::builder()
        .title(s.settings_excluded_titles)
        .description(s.settings_excluded_titles_sub)
        .build();
    let titles: Rc<RefCell<Vec<String>>> =
        Rc::new(RefCell::new(config.privacy.excluded_window_titles.clone()));
    let title_list = gtk::ListBox::new();
    title_list.add_css_class("boxed-list");
    title_list.set_selection_mode(gtk::SelectionMode::None);
    rebuild_list(&title_list, &titles, s.settings_excluded_remove);
    let title_row = adw::EntryRow::builder()
        .title(s.settings_excluded_title_placeholder)
        .show_apply_button(true)
        .build();
    {
        let titles = titles.clone();
        let title_list = title_list.clone();
        let remove_label = s.settings_excluded_remove;
        title_row.connect_apply(move |row| {
            let phrase = row.text().trim().to_string();
            if phrase.is_empty() || phrase.chars().count() > 256 {
                return;
            }
            let mut list = titles.borrow_mut();
            if !list.contains(&phrase) && list.len() < 64 {
                list.push(phrase);
            }
            drop(list);
            row.set_text("");
            rebuild_list(&title_list, &titles, remove_label);
        });
    }
    titles_group.add(&title_row);
    titles_group.add(&title_list);
    page.add(&titles_group);

    // --- interface ---------------------------------------------------
    let interface = adw::PreferencesGroup::builder()
        .title(s.settings_interface)
        .build();
    let language = adw::ComboRow::builder().title(s.settings_language).build();
    language.set_model(Some(&gtk::StringList::new(&[
        s.settings_language_system,
        s.settings_language_tr,
        s.settings_language_en,
    ])));
    language.set_selected(index_of(&LANGUAGES, &config.ui.language));
    interface.add(&language);

    let theme = adw::ComboRow::builder().title(s.settings_theme).build();
    theme.set_model(Some(&gtk::StringList::new(&[
        s.settings_theme_system,
        s.settings_theme_light,
        s.settings_theme_dark,
    ])));
    theme.set_selected(index_of(&THEMES, &config.ui.theme));
    theme.connect_selected_notify(|row| {
        apply_theme(THEMES[row.selected() as usize % THEMES.len()]);
    });
    interface.add(&theme);

    let instant_paste = adw::SwitchRow::builder()
        .title(s.settings_instant_paste)
        .subtitle(s.settings_instant_paste_sub)
        .active(config.ui.instant_paste)
        .build();
    interface.add(&instant_paste);

    let close_on_focus_loss = adw::SwitchRow::builder()
        .title(s.settings_close_on_focus_loss)
        .subtitle(s.settings_close_on_focus_loss_sub)
        .active(config.ui.close_on_focus_loss)
        .build();
    interface.add(&close_on_focus_loss);

    let position = adw::ComboRow::builder()
        .title(s.settings_position)
        .subtitle(s.settings_position_sub)
        .build();
    position.set_model(Some(&gtk::StringList::new(&[
        s.settings_position_pointer,
        s.settings_position_center,
    ])));
    position.set_selected(index_of(POSITIONS, &config.ui.position));
    interface.add(&position);

    let hint = gtk::Label::new(Some(s.settings_restart_hint));
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.set_margin_top(6);
    interface.add(&hint);
    page.add(&interface);

    // --- system ------------------------------------------------------
    let system = adw::PreferencesGroup::builder()
        .title(s.settings_system)
        .build();
    let autostart = adw::SwitchRow::builder()
        .title(s.settings_autostart)
        .subtitle(s.settings_autostart_sub)
        .sensitive(false)
        .build();
    system.add(&autostart);
    let storage = adw::ActionRow::builder()
        .title(s.settings_storage)
        .subtitle(s.settings_storage_sub)
        .build();
    let storage_value = gtk::Label::new(Some("…"));
    storage_value.add_css_class("dim-label");
    storage.add_suffix(&storage_value);
    system.add(&storage);
    page.add(&system);

    // Both answers come from outside the process; neither may stall the
    // dialog, so they arrive when they arrive.
    {
        let autostart = autostart.clone();
        let ui = ui.clone();
        spawn(unit_enabled, move |enabled| {
            let Some(enabled) = enabled else {
                return;
            };
            autostart.set_active(enabled);
            autostart.set_sensitive(true);
            connect_autostart(&ui, &autostart);
        });
    }
    {
        let storage_value = storage_value.clone();
        spawn(
            || dir_size(&data_dir()),
            move |bytes| storage_value.set_text(&format_size(s, bytes)),
        );
    }

    dialog.add(&page);
    let devices = crate::devices::page(s, &dialog);
    dialog.add(&devices);
    if devices_first {
        dialog.set_visible_page(&devices);
    }

    // Save on close: every row above is live state, so there is no separate
    // "apply" step to forget.
    let handler = ui.clone();
    dialog.connect_closed(move |_| {
        let ui = &handler;
        let mut next = ui.app.config.borrow().clone();
        next.history.max_entries = max_entries.value().round() as usize;
        next.history.max_age_days = max_age.value().round() as u32;
        next.history.record_primary = record_primary.is_active();
        next.privacy.start_private = start_private.is_active();
        next.privacy.excluded_apps = excluded.borrow().clone();
        next.privacy.min_text_length = min_text_length.value().round() as usize;
        next.privacy.ignore_whitespace_only = ignore_whitespace.is_active();
        next.privacy.ignore_patterns = patterns.borrow().clone();
        next.privacy.excluded_window_titles = titles.borrow().clone();
        next.privacy.capture_kinds = selected_kinds(&kind_switches);
        next.privacy.sensitive_policy = SENSITIVE_POLICIES
            [sensitive_policy.selected() as usize % SENSITIVE_POLICIES.len()]
        .into();
        next.privacy.sensitive_ttl_minutes = sensitive_ttl.value().round() as u32;
        next.history.index_full_text = index_full_text.is_active();
        next.ui.language = LANGUAGES[language.selected() as usize % LANGUAGES.len()].into();
        next.ui.theme = THEMES[theme.selected() as usize % THEMES.len()].into();
        next.ui.instant_paste = instant_paste.is_active();
        next.ui.close_on_focus_loss = close_on_focus_loss.is_active();
        next.ui.position = POSITIONS[position.selected() as usize % POSITIONS.len()].into();
        save(ui, next);
    });
    dialog.present(Some(&ui.window));
}

fn save(ui: &Rc<Ui>, mut next: Config) {
    let mut current = ui.app.config.borrow().clone();
    // `[sync]` belongs to panora-sync, which turns `enabled` on and off as
    // this device joins or leaves a group -- possibly from this very
    // dialog. Keep what is on disk rather than the copy read at startup.
    if let Ok(on_disk) = Config::load() {
        next.sync = on_disk.sync;
    }
    current.sync = next.sync.clone();
    if same_config(&current, &next) {
        *ui.app.config.borrow_mut() = current;
        return;
    }
    match next.save() {
        Ok(()) => {
            *ui.app.config.borrow_mut() = next;
            apply_theme(&ui.app.config.borrow().ui.theme);
            match call(&Request::ReloadConfig) {
                Ok(_) => toast(ui, ui.s.toast_settings_saved),
                Err(_) => toast(ui, ui.s.toast_settings_saved),
            }
        }
        Err(e) => {
            eprintln!("panora-gui: settings not saved: {e}");
            toast(ui, ui.s.toast_settings_failed);
        }
    }
}

fn same_config(a: &Config, b: &Config) -> bool {
    toml::to_string(a).ok() == toml::to_string(b).ok()
}

fn index_of(options: &[&str], value: &str) -> u32 {
    options.iter().position(|o| *o == value).unwrap_or(0) as u32
}

/// Flipping the switch enables or disables the unit; a failure is shown
/// and the switch goes back to what the system has.
fn connect_autostart(ui: &Rc<Ui>, row: &adw::SwitchRow) {
    let reverting = Rc::new(Cell::new(false));
    let ui = ui.clone();
    row.connect_active_notify(move |row| {
        if reverting.get() {
            return;
        }
        let wanted = row.is_active();
        row.set_sensitive(false);
        let row = row.clone();
        let ui = ui.clone();
        let reverting = reverting.clone();
        spawn(
            move || set_unit_enabled(wanted),
            move |result| {
                row.set_sensitive(true);
                if let Err(e) = result {
                    toast(&ui, &fill(ui.s.settings_autostart_failed, "e", &e));
                    reverting.set(true);
                    row.set_active(!wanted);
                    reverting.set(false);
                }
            },
        );
    });
}

/// `systemctl --user is-enabled panod.service`; `None` when systemd has no
/// answer (no user bus, no unit), in which case the switch stays disabled.
fn unit_enabled() -> Option<bool> {
    let output = std::process::Command::new("systemctl")
        .args(["--user", "is-enabled", "panod.service"])
        .output()
        .ok()?;
    let state = String::from_utf8_lossy(&output.stdout);
    let state = state.trim();
    if state.is_empty() {
        return None;
    }
    Some(matches!(
        state,
        "enabled" | "enabled-runtime" | "static" | "alias" | "linked"
    ))
}

/// Enable or disable the unit for the next login; the running daemon is
/// left alone either way.
fn set_unit_enabled(on: bool) -> Result<(), String> {
    let verb = if on { "enable" } else { "disable" };
    let output = std::process::Command::new("systemctl")
        .args(["--user", verb, "panod.service"])
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Bytes of the regular files under `dir`, symlinks not followed.
fn dir_size(dir: &std::path::Path) -> i64 {
    let mut total = 0i64;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                pending.push(entry.path());
            } else if meta.is_file() {
                total = total.saturating_add(i64::try_from(meta.len()).unwrap_or(i64::MAX));
            }
        }
    }
    total
}

/// The `capture_kinds` value the switches describe: empty (everything) when
/// they are all on, otherwise the kinds left on plus `binary`, which the
/// dialog never offers and must not silently drop.
fn selected_kinds(switches: &[(&'static str, adw::SwitchRow)]) -> Vec<String> {
    let on: Vec<String> = switches
        .iter()
        .filter(|(_, row)| row.is_active())
        .map(|(name, _)| (*name).to_string())
        .collect();
    if on.len() == switches.len() {
        return Vec::new();
    }
    on.into_iter()
        .chain(std::iter::once("binary".into()))
        .collect()
}

/// Refill a boxed list with one removable row per item (excluded
/// applications, ignore patterns).
fn rebuild_list(list: &gtk::ListBox, items: &Rc<RefCell<Vec<String>>>, remove_label: &'static str) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let current = items.borrow().clone();
    for item in current {
        // Patterns can contain `<` and `&`; the title is text, not markup.
        let row = adw::ActionRow::builder()
            .title(&item)
            .use_markup(false)
            .build();
        let remove = gtk::Button::from_icon_name("list-remove-symbolic");
        remove.add_css_class("flat");
        remove.set_tooltip_text(Some(remove_label));
        remove.set_valign(gtk::Align::Center);
        {
            let items = items.clone();
            let list = list.clone();
            let name = item.clone();
            remove.connect_clicked(move |_| {
                items.borrow_mut().retain(|a| a != &name);
                rebuild_list(&list, &items, remove_label);
            });
        }
        row.add_suffix(&remove);
        list.append(&row);
    }
}
