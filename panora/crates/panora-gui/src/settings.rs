// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Preferences dialog. Writes `config.toml` and asks the daemon to reload it
//! so history limits and application exclusions apply immediately.

use crate::util::{apply_theme, call};
use crate::window::{toast, Ui};
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use panora_core::config::Config;
use panora_core::ipc::{Request, ResponseData};
use std::cell::RefCell;
use std::rc::Rc;

const LANGUAGES: [&str; 3] = ["system", "tr", "en"];
const THEMES: [&str; 3] = ["system", "light", "dark"];

/// Open the preferences dialog.
pub fn show(ui: &Rc<Ui>) {
    let s = ui.s;
    let config = ui.app.config.borrow().clone();
    let dialog = adw::PreferencesDialog::builder()
        .title(s.settings_title)
        .build();
    let page = adw::PreferencesPage::new();

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
    rebuild_excluded(&list, &excluded, s.settings_excluded_remove);

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
            rebuild_excluded(&list, &excluded, remove_label);
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

    let hint = gtk::Label::new(Some(s.settings_restart_hint));
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.set_margin_top(6);
    interface.add(&hint);
    page.add(&interface);

    dialog.add(&page);

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
        next.ui.language = LANGUAGES[language.selected() as usize % LANGUAGES.len()].into();
        next.ui.theme = THEMES[theme.selected() as usize % THEMES.len()].into();
        next.ui.instant_paste = instant_paste.is_active();
        save(ui, next);
    });
    dialog.present(Some(&ui.window));
}

fn save(ui: &Rc<Ui>, next: Config) {
    let current = ui.app.config.borrow().clone();
    if same_config(&current, &next) {
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

fn rebuild_excluded(
    list: &gtk::ListBox,
    excluded: &Rc<RefCell<Vec<String>>>,
    remove_label: &'static str,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let apps = excluded.borrow().clone();
    for app in apps {
        let row = adw::ActionRow::builder().title(&app).build();
        let remove = gtk::Button::from_icon_name("list-remove-symbolic");
        remove.add_css_class("flat");
        remove.set_tooltip_text(Some(remove_label));
        remove.set_valign(gtk::Align::Center);
        {
            let excluded = excluded.clone();
            let list = list.clone();
            let name = app.clone();
            remove.connect_clicked(move |_| {
                excluded.borrow_mut().retain(|a| a != &name);
                rebuild_excluded(&list, &excluded, remove_label);
            });
        }
        row.add_suffix(&remove);
        list.append(&row);
    }
}
