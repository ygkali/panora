#!/usr/bin/env bash
# Builds the distributable install kit: kaynak + hazır .deb + tek komutluk
# kurulum sarmalayıcıları, tek bir tar.gz içinde.
#
# Linux üzerinde çalıştırın. Windows'ta paketlerseniz script'lerin çalıştırma
# biti kaybolur ve kullanıcı "permission denied" alır.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)"
KIT="panora-${VERSION}-kurulum-kiti"
OUT_DIR="${1:-$ROOT_DIR/dist}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

echo "[1/4] Debian paketi üretiliyor..."
./packaging/build-deb.sh >/dev/null
DEB="$(find dist -maxdepth 1 -name 'panora_*.deb' -print -quit)"
[[ -n "$DEB" ]] || { echo "Hata: .deb üretilemedi." >&2; exit 1; }

echo "[2/4] Kit ağacı hazırlanıyor..."
install -d "$STAGE/$KIT/panora"
# target/ hariç her şey: kaynak, belgeler, ekran görüntüleri, test script'leri.
tar -C "$ROOT_DIR" --exclude=./target --exclude=./.git -cf - . \
  | tar -C "$STAGE/$KIT/panora" -xf -

for f in KUR.sh TEST.sh KALDIR.sh; do
  case "$f" in
    KUR.sh)    target=install.sh ;;
    TEST.sh)   target=test-local.sh ;;
    KALDIR.sh) target=uninstall.sh ;;
  esac
  cat > "$STAGE/$KIT/$f" <<WRAPPER
#!/usr/bin/env bash
set -Eeuo pipefail
ROOT_DIR="\$(cd -- "\$(dirname -- "\${BASH_SOURCE[0]}")" && pwd)"
exec "\$ROOT_DIR/panora/$target" "\$@"
WRAPPER
  chmod 0755 "$STAGE/$KIT/$f"
done
chmod 0755 "$STAGE/$KIT/panora/install.sh" "$STAGE/$KIT/panora/uninstall.sh" \
           "$STAGE/$KIT/panora/test-local.sh" "$STAGE/$KIT/panora/packaging/"*.sh \
           "$STAGE/$KIT/panora/scripts/"*.sh "$STAGE/$KIT/panora/scripts/panora-doctor" \
           "$STAGE/$KIT/panora/test-artifacts/"*.sh 2>/dev/null || true

echo "[3/4] Kullanım notu yazılıyor..."
cat > "$STAGE/$KIT/BENI-OKU.md" <<'READMEKIT'
# Panora — kurulum ve kullanım

## Kurulum (tek komut)

Arşivi açtığınız klasörde:

```sh
./KUR.sh
```

Sudo parolanız istenir. Script bağımlılıkları kurar, Panora paketini yükler ve
arka plan servisini başlatır. Hazır paket makinenizle uyumsuzsa (eski glibc)
otomatik olarak kaynaktan derlemeye geçer — bunun için Rust gerekir.

## Kullanım

Pencereyi açmak için:

```sh
panora
```

GNOME kullanıyorsanız kısayol **Super+V**'dir. Etkinleştirmek için:

```sh
gnome-extensions enable panora@ygkali.github.io
```

Ardından GNOME'u yeniden başlatın (X11'de `Alt+F2` → `r`, Wayland'de oturumu
kapatıp açın).

### Pencere içi kısayollar

| Kısayol | İşlev |
| --- | --- |
| `Ctrl+F` | Aramaya git |
| `↑ ↓ ← →` | Kayıtlar arasında gez |
| `Enter` | Seçili kaydı panoya koy ve kapat |
| `Ctrl+D` | Sabitle / sabitlemeyi kaldır |
| `Delete` | Seçili kaydı sil |
| `Ctrl+Shift+P` | Özel mod (kayıt almayı durdurur) |
| `Esc` | Aramayı temizle; arama boşsa pencereyi kapat |

## Çalışıyor mu?

```sh
panora-cli status      # backend, kayıt sayısı, özel mod
panora-cli list        # son kayıtlar
```

Bir şey kopyalayıp `panora-cli list` ile göründüğünü doğrulayın.

## Sorun giderme

Servis durumu ve günlükler:

```sh
systemctl --user status panod.service
journalctl --user -u panod.service -n 50 --no-pager
```

Servis başlamıyorsa en sık iki sebep:

- **Ekran değişkenleri eksik.** `systemctl --user import-environment DISPLAY WAYLAND_DISPLAY XDG_SESSION_TYPE` çalıştırıp `systemctl --user restart panod.service` deneyin.
- **Anahtarlık kilitli.** Panora şifreleme anahtarını sistem anahtarlığında tutar; oturumunuzda gnome-keyring'in açık olması gerekir.

## Kaldırma

```sh
./KALDIR.sh
```

Program ve servis kaldırılır, **şifreli geçmişiniz silinmez**. Veriyi de silmek
isterseniz `~/.local/share/panora` ve `~/.config/panora` yollarını ayrıca
kaldırın.

## Bilinmesi gerekenler

- CLI'nin adı `panora-cli`. `panora` komutu arayüzü açar; `panora list`
  yazarsanız GTK "can not open files" der — hata değil, `panora-cli list`
  kullanın.
- Wayland oturumlarında uygulama adına göre dışlama (KeePassXC vb.) çalışmaz;
  Wayland protokolü kopyalayan uygulamanın kimliğini vermiyor. Parola
  yöneticilerinin MIME bayrağına dayanan koruması her iki oturumda da çalışır.
- Ağ erişimi yoktur; systemd unit'i `RestrictAddressFamilies=AF_UNIX` ile bunu
  çekirdek seviyesinde zorlar.
READMEKIT

echo "[4/4] Arşiv paketleniyor..."
install -d "$OUT_DIR"
ARCHIVE="$OUT_DIR/$KIT.tar.gz"
rm -f "$ARCHIVE"
tar -C "$STAGE" -czf "$ARCHIVE" "$KIT"

echo
echo "Kit hazır: $ARCHIVE"
du -h "$ARCHIVE" | cut -f1 | sed 's/^/Boyut: /'
sha256sum "$ARCHIVE"
