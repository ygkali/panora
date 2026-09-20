# Untested starting point, contributed so a Nix packager does not begin from
# an empty file. The project builds and tests only the Debian package;
# nothing here runs in CI and no maintainer has run `nix build` on it.
#
#   nix build .?dir=packaging/nix
#   nix run .?dir=packaging/nix#panora
#
# `cargoHash` is a placeholder: run the build once, take the hash Nix prints
# and put it here.
{
  description = "Panora - encrypted clipboard history for the GNOME desktop";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        appId = "io.github.ygkali.Panora";
        extUuid = "panora@ygkali.github.io";

        panora = pkgs.rustPlatform.buildRustPackage rec {
          pname = "panora";
          version = "1.3.0";

          # Point this at a release tarball for a real package; the relative
          # path keeps `nix build` working from a checkout.
          src = ../..;

          cargoHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

          nativeBuildInputs = with pkgs; [
            pkg-config
            wrapGAppsHook4
            glib # glib-compile-schemas
          ];

          buildInputs = with pkgs; [
            glib
            gtk4
            libadwaita
            sqlite
          ];

          # The X11 and Wayland backend tests need a display and self-skip
          # without one; the keyring test needs a Secret Service and is
          # marked #[ignore].
          doCheck = true;

          postInstall = ''
            install -Dm0755 $src/scripts/panora-doctor $out/bin/panora-doctor
            ln -s panora-gui $out/bin/panora

            # A user unit; NixOS wires it up through
            # systemd.user.services, not through a system preset.
            install -Dm0644 $src/packaging/panod.service \
              $out/lib/systemd/user/panod.service
            install -Dm0644 $src/packaging/${appId}.service \
              $out/share/dbus-1/services/${appId}.service

            install -Dm0644 $src/packaging/${appId}.desktop \
              $out/share/applications/${appId}.desktop
            install -Dm0644 $src/packaging/${appId}.metainfo.xml \
              $out/share/metainfo/${appId}.metainfo.xml
            install -Dm0644 $src/packaging/icons/${appId}.svg \
              $out/share/icons/hicolor/scalable/apps/${appId}.svg
            install -Dm0644 $src/packaging/icons/${appId}-symbolic.svg \
              $out/share/icons/hicolor/symbolic/apps/${appId}-symbolic.svg

            install -d $out/share/man/man1
            $out/bin/panora-cli man $out/share/man/man1
            install -Dm0644 $src/packaging/man/*.1 -t $out/share/man/man1

            installShellCompletion --cmd panora-cli \
              --bash <($out/bin/panora-cli completions bash) \
              --zsh <($out/bin/panora-cli completions zsh) \
              --fish <($out/bin/panora-cli completions fish)

            ext=$out/share/gnome-shell/extensions/${extUuid}
            install -Dm0644 $src/gnome-extension/metadata.json $ext/metadata.json
            install -Dm0644 $src/gnome-extension/extension.js $ext/extension.js
            install -Dm0644 $src/gnome-extension/schemas/*.gschema.xml -t $ext/schemas
            glib-compile-schemas $ext/schemas
          '';

          meta = with pkgs.lib; {
            description = "Encrypted clipboard history for the GNOME desktop";
            homepage = "https://github.com/ygkali/panora";
            license = licenses.gpl3Only;
            platforms = platforms.linux;
            mainProgram = "panora";
          };
        };
      in
      {
        packages.default = panora;
        packages.panora = panora;

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ cargo rustc rustfmt clippy pkg-config glib ];
          buildInputs = with pkgs; [ gtk4 libadwaita sqlite ];
        };
      });
}
