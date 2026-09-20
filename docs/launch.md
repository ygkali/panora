# Launch drafts

Announcement drafts for the 1.3.0 release, one per venue (`DOC-07`). None
of them go out before the real-machine verification in
`docs/RELEASING.md` is recorded and the release is published — a broken
first impression is not worth the day saved.

Two rules for all of them:

- **No superlatives, no claims the code does not back.** "Encrypted at
  rest, key in the Secret Service" is checkable. "Secure clipboard manager"
  is not.
- **Say what it does not do.** Every venue below has readers who will find
  the limitation in ten minutes; better they read it from us. The Wayland
  source-application gap, the GTK 4.12 floor and "no independent audit" go
  in the first few lines, not in a reply.

Fill in `<version>`, `<date>` and the release link before posting.

## Order

1. **GNOME Discourse** and **This Week in GNOME** first — the people who
   will find real bugs, and the ones whose feedback should shape the
   README before a wider audience reads it.
2. **Show HN** and **r/linux** a few days later, once the first issues are
   triaged. Traffic there arrives in one hour and never comes back.
3. **r/gnome**, **Fosstodon** and the Turkish forums alongside.

Be at a keyboard for the first six hours after a Show HN or a subreddit
post. An unanswered "does this work on X?" reads as abandonware.

---

## Show HN

**Title** (80 characters, no exclamation marks, no "revolutionary"):

> Show HN: Panora – encrypted clipboard history for GNOME, in Rust

**Body:**

> I wanted a clipboard manager for GNOME that I could leave running while
> using a password manager, and I could not find one that encrypted its
> history or that refused to read a payload a password manager had marked
> as secret. So Panora does both.
>
> The daemon watches the clipboard through whichever protocol the session
> offers — the X11 selection, `ext-data-control` or `wlr-data-control` on
> Wayland, or a GNOME Shell extension on the GNOME versions that have
> neither. Every offer is judged before a single byte of it is read: the
> MIME list, the source application and the window title decide, because
> that is all you get before you ask for the content. What survives goes
> into a SQLite database encrypted with XChaCha20-Poly1305, with the master
> key in the Secret Service. The daemon is a systemd user service with a
> tight sandbox and no network code at all — CI fails the build if a
> network primitive appears in the daemon or the extension.
>
> What it does not do: on a plain Wayland compositor the protocol does not
> say which application owns the clipboard, so an exclusion list by
> application name cannot fire there — the marker-based gate still does,
> and `panora-cli status` tells you which you are getting. It needs GTK
> 4.12, so Debian 13 / Ubuntu 24.04 and newer only. There has been no
> independent security audit; the threat model and what is outside it are
> written down rather than implied.
>
> GPL-3.0, Debian packages and a signed APT repository for amd64 and
> arm64. Docs: <docs link>. Repository: <repository link>.
>
> Happy to answer anything about the Wayland clipboard protocols — that
> part took far longer than the encryption.

**In the comments, be ready for:** why not Flatpak (portals do not cover
the session bus, Secret Service and clipboard protocols it needs — it is
written up as PKG-04); why another clipboard manager (encryption at rest
and the pre-read gate); why Rust (no interesting answer, say so); "GNOME
already has this" (it does not, the Shell keeps no history); what happens
on X11 versus Wayland (link the desktop matrix).

---

## r/linux

**Title:** `Panora: a clipboard history for Linux desktops that encrypts what it keeps`

Same substance as the Show HN, less compressed, and lead with the desktop
matrix rather than with GNOME — r/linux is not a GNOME audience and "GNOME
clipboard manager" reads as "not for me".

> Panora keeps your clipboard history in an encrypted SQLite database with
> the key in your keyring, and refuses to read anything a password manager
> has flagged as secret — the check happens on the format list, before the
> content is fetched.
>
> It runs on GNOME (Wayland and Xorg), KDE Plasma, Sway, Hyprland and the
> other wlroots compositors, and on Xfce, MATE, Cinnamon and LXQt. What
> works where is a table in the docs rather than a promise: the source
> application of a copy is known on X11, on GNOME through the extension and
> on wlroots or KWin through `wlr-foreign-toplevel-management`, and is not
> known on a plain Wayland compositor. Instant paste needs XTEST or
> `wtype`/`ydotool`. `panora-doctor` reports what your session gives you.
>
> Daemon is a systemd user service, no network code anywhere, no
> telemetry, GPL-3.0. `.deb` for amd64 and arm64 plus a signed APT
> repository; AUR, Nix and RPM starting points are in the tree, untested,
> and corrections are welcome.
>
> Needs GTK 4.12, so Debian 13 / Ubuntu 24.04 / Zorin 18 and newer.
>
> <repository link> — <docs link>

---

## r/gnome

**Title:** `Panora — clipboard history with a Shell extension for Super+V, encrypted at rest`

Shorter, and about the GNOME integration specifically.

> The extension binds Super+V, takes the shortcut over from the
> notification list, and on GNOME 45–47 Wayland is what lets the daemon see
> the clipboard at all — those versions have no data-control protocol. On
> GNOME 48 and newer the daemon uses `ext-data-control` directly and the
> extension is only the shortcut and the paste keystroke.
>
> The history is encrypted with the key in gnome-keyring, the popup is
> GTK4/libadwaita and follows the system theme, and the whole thing is a
> systemd user service. Session-bus only; the extension has no network
> primitives and CI checks that.
>
> Not on extensions.gnome.org yet — `prefs.js` and the review notes are
> the next item. For now it comes with the `.deb`.
>
> <repository link>

---

## GNOME Discourse

Category: **Applications**. Longer, and ask for something specific rather
than announcing.

> **Panora: encrypted clipboard history, and a question about the
> extension's future**
>
> I have been building a clipboard manager for GNOME and it is at its
> first release. The short version: history in an encrypted SQLite file,
> master key in the Secret Service, a Shell extension for Super+V and for
> capture on GNOME versions without `ext-data-control`, no network code
> anywhere.
>
> Two things I would like input on from people who know the Shell better
> than I do.
>
> First, the extension declares `session-modes: ["user", "zorin"]` because
> Zorin OS 18 runs its own session mode. Is that going to be a problem for
> an extensions.gnome.org review, and is there a better way to say "any
> normal user session"?
>
> Second, capture on GNOME 45–47 goes through the extension because there
> is no data-control protocol there; on 48+ the daemon talks the protocol
> itself. I would like to drop the extension path when 47 goes out of
> support. Is there a reason to keep it — something `ext-data-control`
> does not give that `St.Clipboard` does?
>
> Repository and docs: <links>. Bug reports from GNOME 49/50 sessions
> especially welcome; I have verified on 46 and 48.

---

## Fosstodon / Mastodon

Under 500 characters, one image (the popup over a terminal), alt text
mandatory.

> Panora <version>: clipboard history for Linux that encrypts what it
> keeps.
>
> • history in an encrypted SQLite file, key in your keyring
> • refuses content a password manager marks secret, before reading it
> • GNOME, KDE, Sway, Hyprland, Xfce — what works where is documented
> • systemd user service, no network code, GPL-3.0
> • .deb + signed APT repo, amd64 and arm64
>
> <links>
>
> #Linux #GNOME #OpenSource #Rust

**Alt text for the image:** "The Panora popup over a terminal window: a
search box, a list of recent clipboard entries showing their kind, a
preview and which application they came from, and a private-mode switch in
the header bar."

---

## This Week in GNOME

Submitted as a merge request to the TWIG repository, third person, two or
three sentences, one screenshot.

> **Panora**
>
> [Panora](<repository link>) is a clipboard history for GNOME that keeps
> its entries in an encrypted SQLite database, with the master key in the
> Secret Service. A Shell extension binds Super+V and, on GNOME versions
> without `ext-data-control`, forwards clipboard changes to the daemon.
>
> ygkali announces:
>
> > Panora <version> is the first release: encrypted history, a privacy
> > gate that judges a clipboard offer before reading it, instant paste,
> > and Debian packages for amd64 and arm64.

---

## Turkish forums

For Ubuntu-TR, Linux forums and the Zorin community threads in Turkish.
Written for people who will run it on Zorin OS 18 in particular.

> **Panora: şifreli pano geçmişi**
>
> GNOME masaüstü için bir pano geçmişi yazdım. Kopyaladığınız her şey
> anahtarlığınızdaki anahtarla şifrelenmiş bir SQLite dosyasında duruyor;
> parola yöneticilerinin "gizli" olarak işaretlediği içerik ise hiç
> okunmadan reddediliyor — karar, içerik alınmadan önce biçim listesine
> bakılarak veriliyor.
>
> Super+V ile açılıyor. Arayüz Türkçe ve İngilizce; sistem diline göre
> kendiliğinden seçiliyor. Kurulum betiği, tanılama aracı
> (`panora-doctor`) ve test betiği de iki dilli.
>
> Zorin OS 18 ve Ubuntu 24.04 üzerinde çalışıyor; GTK 4.12 gerektirdiği
> için Zorin 17 ve Ubuntu 22.04 desteklenmiyor. Ağ erişimi yok, telemetri
> yok, GPL-3.0.
>
> `.deb` paketi ve imzalı APT deposu: <links>
>
> Hata bildirimleri ve Türkçe çeviri düzeltmeleri için depodaki issue
> bölümü açık.

---

## After posting

- Watch the issue tracker, not the comment threads, for anything that
  looks like a real bug; move it to an issue and answer in the thread with
  the link.
- A crash report in the first week is worth a patch release. Note what
  broke in `docs/verification/` while it is fresh.
- Update `docs/DISTRIBUTION.md` when a community package appears, and link
  it from the README.
