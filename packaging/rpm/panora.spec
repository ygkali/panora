### Untested starting point, contributed so a Fedora/openSUSE packager does
### not begin from an empty file. The project builds and tests only the
### Debian package; nothing here runs in CI and no maintainer has run
### rpmbuild on it. A real submission still needs the bundled-crate
### Provides that %cargo_generate_buildrequires produces.

%global appid   io.github.ygkali.Panora
%global extuuid panora@ygkali.github.io

Name:           panora
Version:        1.3.0
Release:        1%{?dist}
Summary:        Encrypted clipboard history for the GNOME desktop

License:        GPL-3.0-only
URL:            https://github.com/ygkali/panora
Source0:        %{url}/archive/refs/tags/v%{version}.tar.gz#/%{name}-%{version}.tar.gz

ExclusiveArch:  x86_64 aarch64

BuildRequires:  rust >= 1.92
BuildRequires:  cargo
BuildRequires:  pkgconfig
BuildRequires:  pkgconfig(gtk4) >= 4.12
BuildRequires:  pkgconfig(libadwaita-1) >= 1.5
BuildRequires:  pkgconfig(sqlite3)
BuildRequires:  glib2-devel
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib
BuildRequires:  systemd-rpm-macros

Requires:       gtk4 >= 4.12
Requires:       libadwaita >= 1.5
Requires:       adwaita-icon-theme
Recommends:     gnome-keyring
Suggests:       gnome-shell
Suggests:       wl-clipboard

%description
Panora keeps the clipboard history in an encrypted SQLite database whose
master key lives in the Secret Service. The popup opens with Super+V, the
daemon is a systemd user service, and nothing leaves the machine: there is
no network code in the daemon or in the GNOME Shell extension.

Passwords and other sensitive copies are recognised and kept short lived,
private mode pauses recording, and recording stops while the session is
locked.

%prep
%autosetup -n %{name}-%{version}

%build
export CARGO_TARGET_DIR=target
cargo build --release --locked --workspace

%check
# The X11 and Wayland backend tests need a display and self-skip without
# one; the keyring test is #[ignore]d without a Secret Service.
cargo test --release --locked --workspace

%install
install -Dpm0755 target/release/panod       %{buildroot}%{_bindir}/panod
install -Dpm0755 target/release/panora-gui  %{buildroot}%{_bindir}/panora-gui
install -Dpm0755 target/release/panora-cli  %{buildroot}%{_bindir}/panora-cli
install -Dpm0755 scripts/panora-doctor      %{buildroot}%{_bindir}/panora-doctor
ln -s panora-gui %{buildroot}%{_bindir}/panora

# A user unit. It must not end up in %{_unitdir}.
install -Dpm0644 packaging/panod.service \
  %{buildroot}%{_userunitdir}/panod.service
install -Dpm0644 packaging/%{appid}.service \
  %{buildroot}%{_datadir}/dbus-1/services/%{appid}.service

install -Dpm0644 packaging/%{appid}.desktop \
  %{buildroot}%{_datadir}/applications/%{appid}.desktop
install -Dpm0644 packaging/%{appid}.metainfo.xml \
  %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml
install -Dpm0644 packaging/icons/%{appid}.svg \
  %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/%{appid}.svg
install -Dpm0644 packaging/icons/%{appid}-symbolic.svg \
  %{buildroot}%{_datadir}/icons/hicolor/symbolic/apps/%{appid}-symbolic.svg

install -d %{buildroot}%{_mandir}/man1
target/release/panora-cli man %{buildroot}%{_mandir}/man1
install -Dpm0644 packaging/man/panod.1 packaging/man/panora-gui.1 \
  packaging/man/panora-doctor.1 -t %{buildroot}%{_mandir}/man1
ln -s panora-gui.1 %{buildroot}%{_mandir}/man1/panora.1

install -d %{buildroot}%{_datadir}/bash-completion/completions
install -d %{buildroot}%{_datadir}/zsh/site-functions
install -d %{buildroot}%{_datadir}/fish/vendor_completions.d
target/release/panora-cli completions bash \
  > %{buildroot}%{_datadir}/bash-completion/completions/panora-cli
target/release/panora-cli completions zsh \
  > %{buildroot}%{_datadir}/zsh/site-functions/_panora-cli
target/release/panora-cli completions fish \
  > %{buildroot}%{_datadir}/fish/vendor_completions.d/panora-cli.fish

install -Dpm0644 gnome-extension/metadata.json \
  %{buildroot}%{_datadir}/gnome-shell/extensions/%{extuuid}/metadata.json
install -Dpm0644 gnome-extension/extension.js \
  %{buildroot}%{_datadir}/gnome-shell/extensions/%{extuuid}/extension.js
install -Dpm0644 gnome-extension/schemas/*.gschema.xml \
  -t %{buildroot}%{_datadir}/gnome-shell/extensions/%{extuuid}/schemas
glib-compile-schemas \
  %{buildroot}%{_datadir}/gnome-shell/extensions/%{extuuid}/schemas

desktop-file-validate %{buildroot}%{_datadir}/applications/%{appid}.desktop
appstream-util validate-relax --nonet \
  %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml

%files
%license LICENSE
%doc README.md CHANGELOG.md THIRD_PARTY_LICENSES.md
%{_bindir}/panod
%{_bindir}/panora
%{_bindir}/panora-cli
%{_bindir}/panora-doctor
%{_bindir}/panora-gui
%{_userunitdir}/panod.service
%{_datadir}/dbus-1/services/%{appid}.service
%{_datadir}/applications/%{appid}.desktop
%{_metainfodir}/%{appid}.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/%{appid}.svg
%{_datadir}/icons/hicolor/symbolic/apps/%{appid}-symbolic.svg
%{_datadir}/gnome-shell/extensions/%{extuuid}/
%{_datadir}/bash-completion/completions/panora-cli
%{_datadir}/zsh/site-functions/_panora-cli
%{_datadir}/fish/vendor_completions.d/panora-cli.fish
%{_mandir}/man1/pano*.1*

%changelog
* Sat Sep 20 2026 Panora contributors <panora@ygkali.github.io> - 1.3.0-1
- Initial, untested spec; see packaging/README.md.
