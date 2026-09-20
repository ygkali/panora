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
    /// Short name for the private-mode toggle, announced by screen
    /// readers; `private_tooltip` is its longer description.
    pub private_label: &'static str,
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
    /// Banner for the `extension_missing` health code.
    pub health_extension_missing: &'static str,
    pub health_extension_enable: &'static str,
    pub toast_extension_enabled: &'static str,
    /// `{e}` = the error text.
    pub toast_extension_enable_failed: &'static str,
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
    /// Button label on the "deleted" toast.
    pub toast_undo: &'static str,
    pub toast_restored: &'static str,
    pub toast_restore_failed: &'static str,
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
    /// Shown when the active backend cannot name the source application, so
    /// the exclusion list above has nothing to match against.
    pub settings_excluded_unsupported: &'static str,
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
    pub settings_close_on_focus_loss: &'static str,
    pub settings_close_on_focus_loss_sub: &'static str,
    pub settings_filters: &'static str,
    pub settings_filters_sub: &'static str,
    pub settings_min_text_length: &'static str,
    pub settings_min_text_length_sub: &'static str,
    pub settings_ignore_whitespace: &'static str,
    pub settings_index_full_text: &'static str,
    pub settings_index_full_text_sub: &'static str,
    pub settings_capture_kinds: &'static str,
    pub settings_capture_kinds_sub: &'static str,
    pub settings_ignore_patterns: &'static str,
    pub settings_ignore_patterns_sub: &'static str,
    pub settings_ignore_pattern_placeholder: &'static str,
    /// Tooltip on the pattern entry once a pattern failed to compile.
    pub settings_ignore_pattern_invalid: &'static str,
    pub settings_excluded_titles: &'static str,
    pub settings_excluded_titles_sub: &'static str,
    pub settings_excluded_title_placeholder: &'static str,
    /// Tooltip of the lock icon on a flagged row.
    pub sensitive_label: &'static str,
    pub details_sensitive_note: &'static str,
    pub settings_sensitive_policy: &'static str,
    pub settings_sensitive_policy_sub: &'static str,
    pub settings_sensitive_mask: &'static str,
    pub settings_sensitive_drop: &'static str,
    pub settings_sensitive_store: &'static str,
    pub settings_sensitive_ttl: &'static str,
    pub settings_sensitive_ttl_sub: &'static str,
    pub settings_system: &'static str,
    pub settings_autostart: &'static str,
    pub settings_autostart_sub: &'static str,
    /// `{e}` = the error text.
    pub settings_autostart_failed: &'static str,
    pub settings_storage: &'static str,
    pub settings_storage_sub: &'static str,
    pub welcome_title: &'static str,
    pub welcome_shortcut_title: &'static str,
    pub welcome_shortcut_body: &'static str,
    pub welcome_privacy_title: &'static str,
    pub welcome_privacy_body: &'static str,
    pub welcome_session_title: &'static str,
    pub welcome_session_all_good: &'static str,
    pub welcome_session_bridge: &'static str,
    pub welcome_session_no_source_app: &'static str,
    pub welcome_session_no_paste: &'static str,
    pub welcome_session_unknown: &'static str,
    pub welcome_next: &'static str,
    pub welcome_done: &'static str,
    pub details_open_link: &'static str,
    pub details_show_qr: &'static str,
    pub details_qr_title: &'static str,
    pub details_copy_as: &'static str,
    pub details_open_folder: &'static str,
    pub details_save_image: &'static str,
    /// `{p}` = the path written.
    pub toast_saved: &'static str,
    /// `{e}` = the error text.
    pub toast_save_failed: &'static str,
    /// `{e}` = the error text.
    pub toast_open_failed: &'static str,
    pub settings_restart_hint: &'static str,
    pub settings_invalid: &'static str,
    pub cli_ok: &'static str,
    /// `{n}` = affected count.
    pub cli_count: &'static str,
}

static TR: Strings = Strings {
    app_name: "Panora",
    subtitle_history: "pano geçmişi",
    subtitle_no_connection: "bağlantı yok",
    subtitle_connection_problem: "bağlantı sorunu",
    subtitle_count: "{n} kayıt",
    search_placeholder: "Panoda ara…",
    private_tooltip: "Özel mod: yeni kopyalar kaydedilmez (Ctrl+Shift+P)",
    private_label: "Özel mod",
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
    health_extension_missing: "GNOME Shell eklentisi çalışmıyor; bu oturumda hiçbir şey kaydedilmiyor.",
    health_extension_enable: "Etkinleştir",
    toast_extension_enabled: "Eklenti etkinleştirildi",
    toast_extension_enable_failed: "Eklenti etkinleştirilemedi: {e}",
    empty_title: "Pano geçmişi boş",
    empty_filter_title: "Bu filtrede kayıt yok",
    empty_description: "Bir şey kopyaladığında burada görünecek.",
    no_results_title: "Sonuç yok",
    no_results_description: "“{q}” ile eşleşen kayıt bulunamadı.",
    error_connect_title: "Daemon'a bağlanılamadı",
    error_connect_description: "panod çalışmıyor olabilir. Başlatmak için:\nsystemctl --user start panod.service",
    error_daemon_title: "Daemon hata döndürdü",
    retry: "Yeniden dene",
    hint_line: "Enter panoya koy  ·  Space ayrıntı  ·  Esc kapat",
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
    shortcuts_body: "Ctrl+F  ·  aramaya git\n↑ ↓ ← →  ·  kayıtlar arasında gez\nHome / End / PgUp / PgDn  ·  listede atla\nEnter  ·  seçili kaydı panoya koy\nShift+Enter  ·  yalnızca düz metnini panoya koy\nCtrl+1…9  ·  listedeki N. kaydı panoya koy\nSpace  ·  seçili kaydın ayrıntısını aç\nCtrl+D  ·  seçili kaydı sabitle / çöz\nDelete  ·  seçili kaydı sil (geri alınabilir)\nCtrl+Shift+P  ·  özel modu aç / kapat\nCtrl+,  ·  ayarlar\nEsc  ·  aramayı temizle / pencereyi kapat\nHerhangi bir harf  ·  aramaya başla",
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
    toast_undo: "Geri al",
    toast_restored: "Kayıt geri getirildi",
    toast_restore_failed: "Kayıt geri getirilemedi",
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
    settings_excluded_unsupported: "Bu oturumda çalışmaz: Wayland data-control protokolü \
                                    kopyalayan uygulamanın kimliğini bildirmiyor. Parola \
                                    yöneticilerinin gizli içerik işaretleri yine de \
                                    engellenir.",
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
    settings_instant_paste_sub: "Kayıt seçilince odaktaki pencereye Ctrl+V (terminallerde Ctrl+Shift+V) gönder",
    settings_close_on_focus_loss: "Odak kaybında kapat",
    settings_close_on_focus_loss_sub: "Başka bir pencereye geçilince panel kapanır (Win+V gibi)",
    settings_filters: "İçerik filtreleri",
    settings_filters_sub: "Metin okunduktan sonra uygulanır; yukarıdaki uygulama listesine daha önce bakılır",
    settings_min_text_length: "En kısa metin",
    settings_min_text_length_sub: "Bundan kısa metin kaydedilmez (karakter)",
    settings_ignore_whitespace: "Yalnızca boşluktan oluşan metni atla",
    settings_index_full_text: "Metnin tamamında ara",
    settings_index_full_text_sub: "500 karakterlik önizlemenin ötesi de dizinlenir (geçmiş veritabanında saklanır)",
    settings_capture_kinds: "Kaydedilen türler",
    settings_capture_kinds_sub: "Kapatılan tür hiç kaydedilmez",
    settings_ignore_patterns: "Yoksayma kalıpları",
    settings_ignore_patterns_sub: "Düzenli ifadeler; eşleşen metin kaydedilmez (örn. 16 haneli kart numarası için ^\\d{16}$)",
    settings_ignore_pattern_placeholder: "düzenli ifade",
    settings_ignore_pattern_invalid: "Geçerli bir düzenli ifade değil",
    settings_excluded_titles: "Hariç tutulan pencere başlıkları",
    settings_excluded_titles_sub: "Odaktaki pencerenin başlığı bunlardan birini içerirken hiçbir şey kaydedilmez (X11 ve GNOME); başlık saklanmaz",
    settings_excluded_title_placeholder: "başlığın bir parçası (örn. İnternet Bankacılığı)",
    sensitive_label: "Hassas içerik: süresi dolunca silinir",
    details_sensitive_note: "Gizli anahtar, jeton ya da kart/hesap numarasına benziyor: listede maskelenir ve ayarlanan süre sonunda silinir.",
    settings_sensitive_policy: "Gizli anahtarlar ve kart numaraları",
    settings_sensitive_policy_sub: "API anahtarı, jeton, kart ya da IBAN'a benzeyen metne ne yapılır",
    settings_sensitive_mask: "Kaydet, önizlemeyi maskele",
    settings_sensitive_drop: "Kaydetme",
    settings_sensitive_store: "Diğerleri gibi kaydet",
    settings_sensitive_ttl: "Şu kadar dakika sonra sil",
    settings_sensitive_ttl_sub: "0 = diğer kayıtlar gibi tutulur; sabitlenenler her durumda kalır",
    settings_system: "Sistem",
    settings_autostart: "Oturumla birlikte başlat",
    settings_autostart_sub: "Panoyu kaydeden kullanıcı servisi (panod.service)",
    settings_autostart_failed: "Servis ayarı değiştirilemedi: {e}",
    settings_storage: "Kullanılan alan",
    settings_storage_sub: "Geçmiş veritabanı ve şifreli içerikler",
    welcome_title: "Panora'ya hoş geldiniz",
    welcome_shortcut_title: "Super+V",
    welcome_shortcut_body: "Geçmişi her yerden açar. Enter seçili kaydı panoya koyar, Shift+Enter yalnızca düz metni, Ctrl+1…9 ilk satırları seçer.",
    welcome_privacy_title: "Panonuz size ait",
    welcome_privacy_body: "Her şey giriş anahtarlığınızdaki bir anahtarla şifrelenir. Parola yöneticileri hiç okunmaz, gizli anahtarlar maskelenir ve süresi dolunca silinir, özel mod kaydı durdurur.",
    welcome_session_title: "Bu oturum",
    welcome_session_all_good: "Burada her şey çalışır: yakalama, anında yapıştırma ve kaynak uygulama bilgisi kullanılabilir.",
    welcome_session_bridge: "Wayland üzerinde GNOME: yakalama Panora Shell eklentisi üzerinden yapılır; eklenti etkin olmalı.",
    welcome_session_no_source_app: "Bu bileşim yöneticisi kopyalayan uygulamayı bildirmez; hariç tutulan uygulamalar listesi burada işlemez.",
    welcome_session_no_paste: "Anında yapıştırma burada yok; kayıtlar panoya konur, yapıştırmak size kalır.",
    welcome_session_unknown: "Daemon'a henüz ulaşılamıyor; sürerse panora-doctor çalıştırın.",
    welcome_next: "İleri",
    welcome_done: "Başla",
    details_open_link: "Tarayıcıda aç",
    details_show_qr: "QR kod",
    details_qr_title: "Başka bir cihazda açmak için tarayın",
    details_copy_as: "… olarak kopyala",
    details_open_folder: "Klasörü aç",
    details_save_image: "Farklı kaydet…",
    toast_saved: "Kaydedildi: {p}",
    toast_save_failed: "Kaydedilemedi: {e}",
    toast_open_failed: "Açılamadı: {e}",
    settings_restart_hint: "Dil değişikliği pencere yeniden açıldığında uygulanır.",
    settings_invalid: "Geçersiz değer",
    cli_ok: "tamam",
    cli_count: "{n} kayıt işlendi.",
};

static EN: Strings = Strings {
    app_name: "Panora",
    subtitle_history: "clipboard history",
    subtitle_no_connection: "not connected",
    subtitle_connection_problem: "connection problem",
    subtitle_count: "{n} items",
    search_placeholder: "Search clipboard…",
    private_tooltip: "Private mode: new copies are not recorded (Ctrl+Shift+P)",
    private_label: "Private mode",
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
    health_extension_missing: "The GNOME Shell extension is not running, so nothing is recorded.",
    health_extension_enable: "Enable",
    toast_extension_enabled: "Extension enabled",
    toast_extension_enable_failed: "Could not enable the extension: {e}",
    empty_title: "Clipboard history is empty",
    empty_filter_title: "Nothing matches this filter",
    empty_description: "Copy something and it will show up here.",
    no_results_title: "No results",
    no_results_description: "Nothing matches “{q}”.",
    error_connect_title: "Cannot reach the daemon",
    error_connect_description: "panod may not be running. Start it with:\nsystemctl --user start panod.service",
    error_daemon_title: "The daemon returned an error",
    retry: "Retry",
    hint_line: "Enter copy  ·  Space details  ·  Esc close",
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
    shortcuts_body: "Ctrl+F  ·  focus search\n↑ ↓ ← →  ·  move between items\nHome / End / PgUp / PgDn  ·  jump in the list\nEnter  ·  put the selected item on the clipboard\nShift+Enter  ·  put only its plain text on the clipboard\nCtrl+1…9  ·  put the Nth item on the clipboard\nSpace  ·  open the selected item's details\nCtrl+D  ·  pin / unpin the selected item\nDelete  ·  delete the selected item (undo from the toast)\nCtrl+Shift+P  ·  toggle private mode\nCtrl+,  ·  settings\nEsc  ·  clear the search / close the window\nAny letter  ·  start searching",
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
    toast_undo: "Undo",
    toast_restored: "Item restored",
    toast_restore_failed: "Could not restore the item",
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
    settings_excluded_unsupported: "Inactive in this session: the Wayland data-control \
                                    protocol does not report which application made the \
                                    copy. Content a password manager flags as secret is \
                                    still blocked.",
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
    settings_instant_paste_sub: "Send Ctrl+V (Ctrl+Shift+V in terminals) to the focused window after picking an item",
    settings_close_on_focus_loss: "Close when focus leaves",
    settings_close_on_focus_loss_sub: "The panel closes when you switch to another window (like Win+V)",
    settings_filters: "Content filters",
    settings_filters_sub: "Applied once the text has been read; the application list above is checked first",
    settings_min_text_length: "Minimum text length",
    settings_min_text_length_sub: "Shorter text is not recorded (characters)",
    settings_ignore_whitespace: "Skip whitespace-only text",
    settings_index_full_text: "Search the whole text",
    settings_index_full_text_sub: "Text beyond the 500-character preview is indexed too (kept in the history database)",
    settings_capture_kinds: "Recorded kinds",
    settings_capture_kinds_sub: "A kind that is switched off is never recorded",
    settings_ignore_patterns: "Ignore patterns",
    settings_ignore_patterns_sub: "Regular expressions; matching text is not recorded (e.g. ^\\d{16}$ for a 16-digit card number)",
    settings_ignore_pattern_placeholder: "regular expression",
    settings_ignore_pattern_invalid: "Not a valid regular expression",
    settings_excluded_titles: "Excluded window titles",
    settings_excluded_titles_sub: "Nothing is recorded while the focused window's title contains one of these (X11 and GNOME); the title is not stored",
    settings_excluded_title_placeholder: "part of a title (e.g. Online Banking)",
    sensitive_label: "Sensitive content: removed when its time is up",
    details_sensitive_note: "Looks like a key, token or card/account number: masked in the list and removed after the configured time.",
    settings_sensitive_policy: "Secrets and card numbers",
    settings_sensitive_policy_sub: "What happens to text that looks like an API key, token, card or IBAN",
    settings_sensitive_mask: "Record, mask the preview",
    settings_sensitive_drop: "Do not record",
    settings_sensitive_store: "Record like anything else",
    settings_sensitive_ttl: "Remove after (minutes)",
    settings_sensitive_ttl_sub: "0 keeps them like other entries; pinned entries stay either way",
    settings_system: "System",
    settings_autostart: "Start with the session",
    settings_autostart_sub: "The user service that records the clipboard (panod.service)",
    settings_autostart_failed: "Could not change the service: {e}",
    settings_storage: "Storage used",
    settings_storage_sub: "History database and encrypted payloads",
    welcome_title: "Welcome to Panora",
    welcome_shortcut_title: "Super+V",
    welcome_shortcut_body: "Opens the history from anywhere. Enter puts the selected entry on the clipboard, Shift+Enter only its plain text, Ctrl+1…9 pick the first rows.",
    welcome_privacy_title: "Your clipboard stays yours",
    welcome_privacy_body: "Everything is encrypted with a key kept in your login keyring. Password managers are never read, secrets are masked and expire, and private mode pauses recording.",
    welcome_session_title: "This session",
    welcome_session_all_good: "Everything works here: capture, instant paste and the source application are all available.",
    welcome_session_bridge: "GNOME on Wayland: capture goes through the Panora Shell extension, which must be enabled.",
    welcome_session_no_source_app: "This compositor does not name the copying application, so the excluded-applications list cannot apply here.",
    welcome_session_no_paste: "Instant paste is not available here; entries are put on the clipboard for you to paste.",
    welcome_session_unknown: "The daemon is not reachable yet; run panora-doctor if this persists.",
    welcome_next: "Next",
    welcome_done: "Get started",
    details_open_link: "Open in browser",
    details_show_qr: "QR code",
    details_qr_title: "Scan to open on another device",
    details_copy_as: "Copy as…",
    details_open_folder: "Open folder",
    details_save_image: "Save as…",
    toast_saved: "Saved to {p}",
    toast_save_failed: "Could not save: {e}",
    toast_open_failed: "Could not open: {e}",
    settings_restart_hint: "Language changes apply the next time the window opens.",
    settings_invalid: "Invalid value",
    cli_ok: "ok",
    cli_count: "{n} items processed.",
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
