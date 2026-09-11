// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Minimal string catalogue for the GUI and CLI (`ui.language` in config).
//!
//! Two catalogues are compiled in; no gettext runtime dependency. `system`
//! resolves through the usual `LC_ALL` / `LC_MESSAGES` / `LANG` chain.

/// Supported interface languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// Türkçe.
    Turkish,
    /// English.
    English,
}

impl Language {
    /// Resolve the configured language (`system`, `tr`, `en`).
    pub fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "tr" | "tr_tr" | "turkish" => Language::Turkish,
            "en" | "en_us" | "en_gb" | "english" => Language::English,
            _ => Self::from_environment(),
        }
    }

    /// Detect from the process locale environment. Turkish locales select
    /// Turkish; everything else falls back to English.
    pub fn from_environment() -> Self {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
            if let Ok(value) = std::env::var(var) {
                if let Some(lang) = Self::from_locale(&value) {
                    return lang;
                }
            }
        }
        Language::English
    }

    /// Parse a locale string such as `tr_TR.UTF-8` or `en_US:en`.
    pub fn from_locale(value: &str) -> Option<Self> {
        let first = value.split(':').next()?.trim();
        if first.is_empty() || first == "C" || first == "POSIX" {
            return None;
        }
        let code = first
            .split(['_', '.', '@', '-'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match code.as_str() {
            "tr" => Some(Language::Turkish),
            "" => None,
            _ => Some(Language::English),
        }
    }

    /// Stable config form.
    pub fn code(self) -> &'static str {
        match self {
            Language::Turkish => "tr",
            Language::English => "en",
        }
    }

    /// The string catalogue for this language.
    pub fn strings(self) -> &'static Strings {
        match self {
            Language::Turkish => &TR,
            Language::English => &EN,
        }
    }
}

/// All user-visible strings. Placeholders are documented per field.
#[allow(missing_docs)]
#[derive(Debug)]
pub struct Strings {
    pub app_name: &'static str,
    pub subtitle_history: &'static str,
    pub subtitle_no_connection: &'static str,
    pub subtitle_connection_problem: &'static str,
    /// `{n}` = entry count.
    pub subtitle_count: &'static str,
    pub search_placeholder: &'static str,
    pub private_tooltip: &'static str,
    pub menu_tooltip: &'static str,
    pub filter_all: &'static str,
    pub filter_pinned: &'static str,
    pub filter_text: &'static str,
    pub filter_link: &'static str,
    pub filter_image: &'static str,
    pub filter_files: &'static str,
    pub filter_richtext: &'static str,
    pub filter_color: &'static str,
    pub banner_private: &'static str,
    pub empty_title: &'static str,
    pub empty_filter_title: &'static str,
    pub empty_description: &'static str,
    pub no_results_title: &'static str,
    /// `{q}` = search query.
    pub no_results_description: &'static str,
    pub error_connect_title: &'static str,
    pub error_connect_description: &'static str,
    pub error_daemon_title: &'static str,
    pub retry: &'static str,
    pub hint_line: &'static str,
    pub menu_shortcuts: &'static str,
    pub menu_settings: &'static str,
    pub menu_clear: &'static str,
    pub clear_title: &'static str,
    pub clear_body: &'static str,
    pub cancel: &'static str,
    pub clear: &'static str,
    pub ok: &'static str,
    pub close: &'static str,
    pub save: &'static str,
    pub shortcuts_title: &'static str,
    pub shortcuts_body: &'static str,
    pub toast_private_on: &'static str,
    pub toast_private_off: &'static str,
    pub toast_private_failed: &'static str,
    /// `{n}` = cleared count.
    pub toast_cleared_n: &'static str,
    pub toast_cleared: &'static str,
    pub toast_clear_failed: &'static str,
    pub toast_recall_failed: &'static str,
    pub toast_paste_failed: &'static str,
    pub toast_pinned: &'static str,
    pub toast_unpinned: &'static str,
    pub toast_pin_failed: &'static str,
    pub toast_deleted: &'static str,
    pub toast_delete_failed: &'static str,
    pub toast_settings_saved: &'static str,
    pub toast_settings_failed: &'static str,
    pub toast_copied_text: &'static str,
    pub kind_text: &'static str,
    pub kind_richtext: &'static str,
    pub kind_link: &'static str,
    pub kind_image: &'static str,
    pub kind_files: &'static str,
    pub kind_color: &'static str,
    pub kind_binary: &'static str,
    pub time_just_now: &'static str,
    /// `{n}` = minutes.
    pub time_minutes: &'static str,
    /// `{n}` = hours.
    pub time_hours: &'static str,
    /// `{n}` = days.
    pub time_days: &'static str,
    /// `{n}` = weeks.
    pub time_weeks: &'static str,
    pub tooltip_click_to_copy: &'static str,
    pub tooltip_pin: &'static str,
    pub tooltip_unpin: &'static str,
    pub tooltip_delete: &'static str,
    pub tooltip_details: &'static str,
    pub details_title: &'static str,
    pub details_copy: &'static str,
    pub details_copy_plain: &'static str,
    pub details_no_text: &'static str,
    pub details_formats: &'static str,
    pub load_more: &'static str,
    pub settings_title: &'static str,
    pub settings_history: &'static str,
    pub settings_max_entries: &'static str,
    pub settings_max_entries_sub: &'static str,
    pub settings_max_age: &'static str,
    pub settings_max_age_sub: &'static str,
    pub settings_record_primary: &'static str,
    pub settings_record_primary_sub: &'static str,
    pub settings_privacy: &'static str,
    pub settings_start_private: &'static str,
    pub settings_excluded_apps: &'static str,
    pub settings_excluded_apps_sub: &'static str,
    pub settings_excluded_add_placeholder: &'static str,
    pub settings_excluded_add: &'static str,
    pub settings_excluded_remove: &'static str,
    pub settings_interface: &'static str,
    pub settings_language: &'static str,
    pub settings_language_system: &'static str,
    pub settings_language_tr: &'static str,
    pub settings_language_en: &'static str,
    pub settings_theme: &'static str,
    pub settings_theme_system: &'static str,
    pub settings_theme_light: &'static str,
    pub settings_theme_dark: &'static str,
    pub settings_instant_paste: &'static str,
    pub settings_instant_paste_sub: &'static str,
    pub settings_restart_hint: &'static str,
    pub settings_invalid: &'static str,
    pub cli_help: &'static str,
    pub cli_ok: &'static str,
    /// `{n}` = affected count.
    pub cli_count: &'static str,
    pub cli_unknown_command: &'static str,
}

static TR: Strings = Strings {
    app_name: "Panora",
    subtitle_history: "pano geçmişi",
    subtitle_no_connection: "bağlantı yok",
    subtitle_connection_problem: "bağlantı sorunu",
    subtitle_count: "{n} kayıt",
    search_placeholder: "Panoda ara…",
    private_tooltip: "Özel mod: yeni kopyalar kaydedilmez (Ctrl+Shift+P)",
    menu_tooltip: "Menü",
    filter_all: "Tümü",
    filter_pinned: "Sabitli",
    filter_text: "Metin",
    filter_link: "Bağlantı",
    filter_image: "Görsel",
    filter_files: "Dosya",
    filter_richtext: "Biçimli",
    filter_color: "Renk",
    banner_private: "Özel mod açık — yeni kopyalar kaydedilmiyor",
    empty_title: "Pano geçmişi boş",
    empty_filter_title: "Bu filtrede kayıt yok",
    empty_description: "Bir şey kopyaladığında burada görünecek.",
    no_results_title: "Sonuç yok",
    no_results_description: "“{q}” ile eşleşen kayıt bulunamadı.",
    error_connect_title: "Daemon'a bağlanılamadı",
    error_connect_description: "panod çalışmıyor olabilir. Başlatmak için:\nsystemctl --user start panod.service",
    error_daemon_title: "Daemon hata döndürdü",
    retry: "Yeniden dene",
    hint_line: "↑ ↓ gez  ·  Enter panoya koy  ·  Space ayrıntı  ·  Ctrl+D sabitle  ·  Delete sil  ·  Esc kapat",
    menu_shortcuts: "Klavye kısayolları",
    menu_settings: "Ayarlar",
    menu_clear: "Geçmişi temizle",
    clear_title: "Geçmiş temizlensin mi?",
    clear_body: "Sabitlenmemiş kayıtlar kalıcı olarak silinir; sabitlediklerin kalır. Bu işlem geri alınamaz.",
    cancel: "Vazgeç",
    clear: "Temizle",
    ok: "Tamam",
    close: "Kapat",
    save: "Kaydet",
    shortcuts_title: "Klavye kısayolları",
    shortcuts_body: "Ctrl+F  ·  aramaya git\n↑ ↓ ← →  ·  kayıtlar arasında gez\nEnter  ·  seçili kaydı panoya koy\nSpace  ·  seçili kaydın ayrıntısını aç\nCtrl+D  ·  seçili kaydı sabitle / çöz\nDelete  ·  seçili kaydı sil\nCtrl+Shift+P  ·  özel modu aç / kapat\nCtrl+,  ·  ayarlar\nEsc  ·  aramayı temizle / pencereyi kapat",
    toast_private_on: "Özel mod açıldı — kayıt durduruldu",
    toast_private_off: "Özel mod kapatıldı",
    toast_private_failed: "Özel mod değiştirilemedi",
    toast_cleared_n: "{n} kayıt silindi",
    toast_cleared: "Geçmiş temizlendi",
    toast_clear_failed: "Geçmiş temizlenemedi",
    toast_recall_failed: "Panoya konulamadı",
    toast_paste_failed: "Panoya kondu, ancak otomatik yapıştırma bu oturumda desteklenmiyor",
    toast_pinned: "Sabitlendi",
    toast_unpinned: "Sabitleme kaldırıldı",
    toast_pin_failed: "Sabitleme değiştirilemedi",
    toast_deleted: "Kayıt silindi",
    toast_delete_failed: "Kayıt silinemedi",
    toast_settings_saved: "Ayarlar kaydedildi",
    toast_settings_failed: "Ayarlar kaydedilemedi",
    toast_copied_text: "Düz metin panoya kondu",
    kind_text: "METİN",
    kind_richtext: "BİÇİMLİ",
    kind_link: "BAĞLANTI",
    kind_image: "GÖRSEL",
    kind_files: "DOSYA",
    kind_color: "RENK",
    kind_binary: "İKİLİ",
    time_just_now: "az önce",
    time_minutes: "{n} dk önce",
    time_hours: "{n} sa önce",
    time_days: "{n} gün önce",
    time_weeks: "{n} hafta önce",
    tooltip_click_to_copy: "Panoya koymak için tıkla",
    tooltip_pin: "Sabitle (Ctrl+D)",
    tooltip_unpin: "Sabitlemeyi kaldır (Ctrl+D)",
    tooltip_delete: "Sil (Delete)",
    tooltip_details: "Ayrıntılar (Space)",
    details_title: "Kayıt ayrıntısı",
    details_copy: "Panoya koy",
    details_copy_plain: "Düz metin olarak koy",
    details_no_text: "Bu kaydın metin önizlemesi yok.",
    details_formats: "Biçimler",
    load_more: "Daha fazla yükle",
    settings_title: "Ayarlar",
    settings_history: "Geçmiş",
    settings_max_entries: "En fazla kayıt",
    settings_max_entries_sub: "Sabitlenmemiş kayıt üst sınırı; eskiler silinir",
    settings_max_age: "Saklama süresi (gün)",
    settings_max_age_sub: "0 = süresiz",
    settings_record_primary: "Seçim panosunu kaydet (PRIMARY)",
    settings_record_primary_sub: "Fareyle seçilen metni de geçmişe ekle",
    settings_privacy: "Gizlilik",
    settings_start_private: "Özel modda başla",
    settings_excluded_apps: "Hariç tutulan uygulamalar",
    settings_excluded_apps_sub: "Bu uygulamalardan kopyalananlar hiç okunmaz",
    settings_excluded_add_placeholder: "uygulama adı (örn. keepassxc)",
    settings_excluded_add: "Ekle",
    settings_excluded_remove: "Kaldır",
    settings_interface: "Arayüz",
    settings_language: "Dil",
    settings_language_system: "Sistem",
    settings_language_tr: "Türkçe",
    settings_language_en: "English",
    settings_theme: "Tema",
    settings_theme_system: "Sistem",
    settings_theme_light: "Açık",
    settings_theme_dark: "Koyu",
    settings_instant_paste: "Anında yapıştır",
    settings_instant_paste_sub: "Kayıt seçilince odaktaki pencereye Ctrl+V gönder",
    settings_restart_hint: "Dil değişikliği pencere yeniden açıldığında uygulanır.",
    settings_invalid: "Geçersiz değer",
    cli_help: "Panora güvenli pano yöneticisi CLI\nKullanım:\n  panora-cli list [arama] [--kind tür] [--pinned] [--limit N] [--offset N]\n  panora-cli search <metin>\n  panora-cli copy <id> [--paste] [--mime tür]\n  panora-cli preview <id> [--mime tür] [--out dosya]\n  panora-cli pin|unpin <id>\n  panora-cli delete <id>\n  panora-cli clear            (sabitlenmemiş kayıtları siler)\n  panora-cli private on|off\n  panora-cli status\n  panora-cli toggle           (popup'ı aç/kapat)\n  panora-cli reload           (config.toml'u yeniden yükle)\n  panora-cli --json ...       (makine okunur çıktı)",
    cli_ok: "tamam",
    cli_count: "{n} kayıt işlendi.",
    cli_unknown_command: "bilinmeyen komut",
};

static EN: Strings = Strings {
    app_name: "Panora",
    subtitle_history: "clipboard history",
    subtitle_no_connection: "not connected",
    subtitle_connection_problem: "connection problem",
    subtitle_count: "{n} items",
    search_placeholder: "Search clipboard…",
    private_tooltip: "Private mode: new copies are not recorded (Ctrl+Shift+P)",
    menu_tooltip: "Menu",
    filter_all: "All",
    filter_pinned: "Pinned",
    filter_text: "Text",
    filter_link: "Links",
    filter_image: "Images",
    filter_files: "Files",
    filter_richtext: "Rich text",
    filter_color: "Colors",
    banner_private: "Private mode is on — new copies are not recorded",
    empty_title: "Clipboard history is empty",
    empty_filter_title: "Nothing matches this filter",
    empty_description: "Copy something and it will show up here.",
    no_results_title: "No results",
    no_results_description: "Nothing matches “{q}”.",
    error_connect_title: "Cannot reach the daemon",
    error_connect_description: "panod may not be running. Start it with:\nsystemctl --user start panod.service",
    error_daemon_title: "The daemon returned an error",
    retry: "Retry",
    hint_line: "↑ ↓ navigate  ·  Enter copy  ·  Space details  ·  Ctrl+D pin  ·  Delete remove  ·  Esc close",
    menu_shortcuts: "Keyboard shortcuts",
    menu_settings: "Settings",
    menu_clear: "Clear history",
    clear_title: "Clear the history?",
    clear_body: "Unpinned items are deleted permanently; pinned items stay. This cannot be undone.",
    cancel: "Cancel",
    clear: "Clear",
    ok: "OK",
    close: "Close",
    save: "Save",
    shortcuts_title: "Keyboard shortcuts",
    shortcuts_body: "Ctrl+F  ·  focus search\n↑ ↓ ← →  ·  move between items\nEnter  ·  put the selected item on the clipboard\nSpace  ·  open the selected item's details\nCtrl+D  ·  pin / unpin the selected item\nDelete  ·  delete the selected item\nCtrl+Shift+P  ·  toggle private mode\nCtrl+,  ·  settings\nEsc  ·  clear the search / close the window",
    toast_private_on: "Private mode on — recording paused",
    toast_private_off: "Private mode off",
    toast_private_failed: "Could not change private mode",
    toast_cleared_n: "{n} items deleted",
    toast_cleared: "History cleared",
    toast_clear_failed: "Could not clear the history",
    toast_recall_failed: "Could not put the item on the clipboard",
    toast_paste_failed: "Copied, but automatic paste is not supported in this session",
    toast_pinned: "Pinned",
    toast_unpinned: "Unpinned",
    toast_pin_failed: "Could not change the pin",
    toast_deleted: "Item deleted",
    toast_delete_failed: "Could not delete the item",
    toast_settings_saved: "Settings saved",
    toast_settings_failed: "Could not save settings",
    toast_copied_text: "Plain text copied",
    kind_text: "TEXT",
    kind_richtext: "RICH",
    kind_link: "LINK",
    kind_image: "IMAGE",
    kind_files: "FILES",
    kind_color: "COLOR",
    kind_binary: "BINARY",
    time_just_now: "just now",
    time_minutes: "{n} min ago",
    time_hours: "{n} h ago",
    time_days: "{n} d ago",
    time_weeks: "{n} w ago",
    tooltip_click_to_copy: "Click to put on the clipboard",
    tooltip_pin: "Pin (Ctrl+D)",
    tooltip_unpin: "Unpin (Ctrl+D)",
    tooltip_delete: "Delete (Delete)",
    tooltip_details: "Details (Space)",
    details_title: "Item details",
    details_copy: "Copy",
    details_copy_plain: "Copy as plain text",
    details_no_text: "This item has no text preview.",
    details_formats: "Formats",
    load_more: "Load more",
    settings_title: "Settings",
    settings_history: "History",
    settings_max_entries: "Maximum items",
    settings_max_entries_sub: "Unpinned item limit; the oldest are removed",
    settings_max_age: "Keep for (days)",
    settings_max_age_sub: "0 = forever",
    settings_record_primary: "Record the selection clipboard (PRIMARY)",
    settings_record_primary_sub: "Also keep text selected with the mouse",
    settings_privacy: "Privacy",
    settings_start_private: "Start in private mode",
    settings_excluded_apps: "Excluded applications",
    settings_excluded_apps_sub: "Copies from these apps are never read",
    settings_excluded_add_placeholder: "application name (e.g. keepassxc)",
    settings_excluded_add: "Add",
    settings_excluded_remove: "Remove",
    settings_interface: "Interface",
    settings_language: "Language",
    settings_language_system: "System",
    settings_language_tr: "Türkçe",
    settings_language_en: "English",
    settings_theme: "Theme",
    settings_theme_system: "System",
    settings_theme_light: "Light",
    settings_theme_dark: "Dark",
    settings_instant_paste: "Instant paste",
    settings_instant_paste_sub: "Send Ctrl+V to the focused window after picking an item",
    settings_restart_hint: "Language changes apply the next time the window opens.",
    settings_invalid: "Invalid value",
    cli_help: "Panora secure clipboard manager CLI\nUsage:\n  panora-cli list [query] [--kind kind] [--pinned] [--limit N] [--offset N]\n  panora-cli search <text>\n  panora-cli copy <id> [--paste] [--mime tür]\n  panora-cli preview <id> [--mime type] [--out file]\n  panora-cli pin|unpin <id>\n  panora-cli delete <id>\n  panora-cli clear            (deletes unpinned items)\n  panora-cli private on|off\n  panora-cli status\n  panora-cli toggle           (show/hide the popup)\n  panora-cli reload           (re-read config.toml)\n  panora-cli --json ...       (machine-readable output)",
    cli_ok: "ok",
    cli_count: "{n} items processed.",
    cli_unknown_command: "unknown command",
};

/// Replace the first `{key}` placeholder with `value`.
pub fn fill(template: &str, key: &str, value: &str) -> String {
    template.replacen(&format!("{{{key}}}"), value, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_parsing() {
        assert_eq!(
            Language::from_locale("tr_TR.UTF-8"),
            Some(Language::Turkish)
        );
        assert_eq!(
            Language::from_locale("en_US.UTF-8"),
            Some(Language::English)
        );
        assert_eq!(Language::from_locale("de_DE"), Some(Language::English));
        assert_eq!(Language::from_locale("C"), None);
        assert_eq!(Language::from_locale(""), None);
        assert_eq!(Language::from_locale("tr:en"), Some(Language::Turkish));
    }

    #[test]
    fn config_codes() {
        assert_eq!(Language::from_config("tr"), Language::Turkish);
        assert_eq!(Language::from_config("EN"), Language::English);
        assert_eq!(Language::Turkish.code(), "tr");
    }

    #[test]
    fn fill_replaces_placeholder() {
        assert_eq!(fill("{n} kayıt", "n", "3"), "3 kayıt");
        assert_eq!(
            fill(EN.no_results_description, "q", "x"),
            "Nothing matches “x”."
        );
    }

    #[test]
    fn catalogues_have_no_empty_strings() {
        for strings in [&TR, &EN] {
            let dump = format!("{strings:?}");
            assert!(!dump.contains("\"\""), "empty string in catalogue");
        }
    }
}
