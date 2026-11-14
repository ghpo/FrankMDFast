<p align="center">
  <img src="assets/brand/icon.svg" width="160" height="160" alt="FrankMD icon">
</p>

<h1 align="center">FrankMDFast</h1>

<p align="center">
  <strong>Your notes, everywhere.</strong><br>
  A blazing-fast, offline-first Markdown vault with native desktop, browser, Android and iOS
  clients — one Rust core, one encrypted vault.
</p>

<p align="center">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-2024-dea584?logo=rust&logoColor=white">
  <img alt="Desktop" src="https://img.shields.io/badge/Desktop-Linux%20%7C%20macOS%20%7C%20Windows-4c9eeb">
  <img alt="Mobile" src="https://img.shields.io/badge/Mobile-Android%20%7C%20iOS-3ddc84">
  <img alt="Remote" src="https://img.shields.io/badge/Remote-Tailscale-8b5cf6">
  <img alt="License" src="https://img.shields.io/badge/License-MIT-yellow">
</p>

> **Fork notice** — This is a fork of
> [akitaonrails/FrankMDFast](https://github.com/akitaonrails/FrankMDFast),
> maintained by [ghpo](https://github.com/ghpo). Upstream is the original work;
> this fork adds opt-in Tailscale remote access, native Android and iOS clients,
> vault sync and the web client. It is **not affiliated with or endorsed by**
> the upstream author. See [FORK.md](FORK.md) for differences, sync and
> reporting rules.

<p align="center">
  <img src="docs/screenshots/main.png" alt="FrankMD with a note in the editor and its live preview, following the Omarchy Tokyo Night theme">
</p>

| Command palette | Settings | Light theme |
|---|---|---|
| ![Command palette](docs/screenshots/palette.png) | ![Settings](docs/screenshots/settings.png) | ![Catppuccin Latte theme](docs/screenshots/light.png) |

## One vault. Four clients.

Your notes live as plain Markdown files in a folder you own. Open the same vault
in the native desktop app, in a browser over Tailscale, or in the Android and
iOS apps — with the same themes and the same encrypted seal.

| Feature | Desktop | Web (Tailscale) | Android | iOS |
|---|---|---|---|---|
| **Markdown editor + live preview** | ✅ | ✅ | ✅ | ✅ |
| **Full-text search** | ✅ | ✅ | ✅ | ✅ |
| **Works offline** | ✅ | — (served by desktop) | ✅ | ✅ |
| **Encrypted notes (shared seal)** | ✅ | ✅ | ✅ | ✅ |
| **Encrypted attachments** | ✅ | ✅ | ✅ | 🚧 |
| **Sync with the desktop vault** | hosts the server | — | every 15 min + on open | on demand |
| **Themes** | 18 + live Omarchy | light / dark / auto | light / dark / auto | light / dark / auto |
| **AI (DeepSeek, OpenAI, Gemini, OpenRouter, Ollama)** | ✅ | ✅ | — | — |
| **PDF / share** | PDF + print | — | share PDF / text | — |
| **Native app** | ✅ | browser only | ✅ | ✅ |

## What's new in this fork

**A real remote client, not a compromise.** The desktop serves the vault over
your Tailscale network with mandatory token auth, live updates over SSE and a
fully responsive web editor. It keeps serving after the window closes and can
run headless as a systemd user service.

**Native Android.** Kotlin + Jetpack Compose on top of the same Rust core
through JNI: notes tree, editor, live preview, full-text search, encryption
unlock, PDF share and automatic sync every 15 minutes plus one sync on app
open.

**Native iOS.** SwiftUI on top of the Rust C ABI, built with
[xtool](https://github.com/xtool-org/xtool) on Linux: local vault, editor,
preview, attachments, search, seal import/unlock/lock and on-demand sync.

**Encryption that travels with you.** Notes and attachments are sealed with
Argon2id + XChaCha20-Poly1305. Import the seal on a phone, unlock with the same
password and the same vault opens — filenames stay visible, content stays
private.

**Sync without the cloud.** The desktop is the server; the phones are clients.
A manifest/revision protocol copies only what changed, keeps conflict copies
when both sides edited the same file, and never requires a third-party account.

**The desktop keeps getting better.** GPUI + gpui-kit, 18 built-in themes that
follow Omarchy live, Vim mode, command palette, PDF/print, media library,
Hugo posts and AI actions — all in a native Rust binary.

## Encryption at rest

Encrypt **selected notes and media** with one vault password
(Argon2id + XChaCha20-Poly1305). Filenames stay visible; content is sealed.
Create/unlock/lock and back up the seal in **Settings › Encryption**; right-click
a note to **Encrypt/Decrypt** — the note and every local attachment it references
are encrypted/decrypted together on desktop, web and Android. Use **Encrypt
attachments of encrypted notes** for notes encrypted by older versions, and the
media library for standalone files. Shared attachments stay encrypted while
another encrypted note uses them; with the vault unlocked, encrypted images are
rendered through a private temporary cache. The paperclip in the preview header
lists every local attachment of the note (images, video, audio, PDFs, archives)
with **Save as…** and **Save all…**.

On mobile, import the seal exported by the desktop, unlock with the same
password and encrypt/decrypt notes in place. iOS note encryption is available
today; attachment encryption is the remaining piece. See
[docs/encryption.md](docs/encryption.md) for the format, key backup, password
rotation and limits.

<p align="center">
  <img src="docs/screenshots/encryption-settings.png" width="32%" alt="Settings › Encryption with the vault locked, unlock field, seal backup and import controls">
  <img src="docs/screenshots/encryption-locked-note.png" width="32%" alt="A locked encrypted note showing the padlock view in the editor and preview">
  <img src="docs/screenshots/encryption-unlock.png" width="32%" alt="Quick unlock dialog for encrypted content">
</p>

## Sync without the cloud

The desktop hosts a small authenticated HTTP API:

- `GET /api/sync/manifest` — every vault file with its SHA-256 revision.
- `GET /api/sync/file` — raw bytes (never decrypted server-side).
- `PUT /api/sync/file` — upload a changed file.
- `DELETE /api/sync/file` — remove a file deleted locally.

Clients compare the remote manifest with their local revisions and a saved base
revision, then download, upload or delete only the differences. When both sides
changed the same path, the sync keeps a conflict copy instead of silently
overwriting. The transport is plain HTTP over Tailscale, protected by the
access token — put it behind a trusted network, not the public internet.

## Remote web client

<p align="center">
  <img src="docs/remote-settings-copy.png" width="860" alt="Settings › Remote access: start on app launch, Tailscale/Loopback, port, running status, generate/save/copy access token and the .fed destination">
</p>

Serve this vault to a browser on another device on your Tailscale network,
without leaving the desktop app:

- **Settings › Remote → Start on app launch** — a per-machine preference that
  starts the server immediately and on every next launch (Tailscale or
  Loopback, editable port).
- **Access token right in the panel** — paste an existing token or click
  **Generate token**, then **Save token** and **Copy token**. The panel shows
  the exact destination: `.fed` (This vault) or the global config (All vaults);
  `FRANKMD_AUTH_TOKEN` overrides the file.
- **It keeps serving after you close the window** — the server runs as a
  detached `frankmdfast --serve-only` daemon; `SUPER+W` does not stop it.
  Settings › Remote has explicit **Start server** / **Stop server** buttons.
- **Start at login** — enable the systemd user unit from Settings › Remote to
  start the server when you log in. For boot before login, enable lingering.
- The browser client has the file tree, autosave, live Markdown preview,
  search, settings, themes, media library/upload, AI actions and encryption
  unlock.
- Authentication is a mandatory token of at least 32 bytes (`auth_token` in
  `.fed`, `FRANKMD_AUTH_TOKEN`, or `--serve-token`).
- Live updates over SSE. The same server can run headless (`--serve-only`) for
  a systemd user unit.

```bash
export FRANKMD_AUTH_TOKEN="$(openssl rand -hex 32)"
frankmdfast --serve ~/notes        # desktop window + remote server
# or headless: frankmdfast --serve-only ~/notes
# then open http://<tailscale-ip>:8787 and paste the token
```

The mobile web layout is a dedicated single-column mode: notes drawer,
editor/preview toggle, save, **Encrypt/Decrypt note**, content lock/unlock and
logout. **Remember this device** keeps the HttpOnly token cookie for one year;
uncheck it for a session-only cookie. The page uses a native-style fixed
layout, disables pinch zoom/page pan and keeps the notes drawer above the
content.

**From another machine:** install Tailscale there, open
`http://<tailscale-ip>:8787` in a browser and paste the token. No FrankMDFast
install is needed — the browser is the remote client. For a native client, use
the Android or iOS app.

The default bind is the Tailscale address; `0.0.0.0`/LAN requires
`--serve-bind-any`. See [docs/remote.md](docs/remote.md) for the security
model, HTTP API and the systemd unit.

## Desktop

The original experience, rewritten in Rust with GPUI + gpui-kit:

- Markdown editor with live preview, math, syntax highlighting, tables, tasks
  and wikilinks.
- Vim mode, typewriter mode, find and replace, command palette
  (`Ctrl+Shift+P`) and quick open (`Ctrl+P`).
- Hugo blog posts, image and video embedding (local folder or S3), YouTube and
  image search.
- AI grammar check and image generation with OpenAI, Anthropic, Gemini,
  OpenRouter or Ollama — including DeepSeek Chat and DeepSeek Reasoner.
- Follows the Omarchy theme and language live; 18 built-in themes.
- Settings for every option, including interface size (80–200%).
- Copy formatted, save PDF and print, all from the preview header.

## Android

A native Kotlin/Compose client for an app-private vault:

- notes tree, create/open/save/delete, editor and live preview;
- notes read/written through the Rust core (`frankmd-mobile` JNI bridge →
  `frankmd-fs`);
- full-text local search (title, path and content; locked notes match by title);
- seal import, unlock/lock, encrypt/decrypt notes and their referenced
  attachments;
- automatic WorkManager sync every 15 minutes (network constrained) plus one
  sync on app start, against the desktop `/api/sync/*` endpoints;
- share a note as PDF or plain text;
- compact top bar, overflow actions and a bottom Edit/Preview/Save bar.

Build:

```bash
cd android
./build-rust.sh
export ANDROID_HOME="$HOME/Android/Sdk"
./gradlew :app:assembleDebug
# APK: app/build/outputs/apk/debug/app-debug.apk
```

## iOS

A native SwiftUI client compiled with
[xtool](https://github.com/xtool-org/xtool) on Linux:

- SwiftUI notes drawer, local vault in `Documents/vault`;
- create/open/edit/save/delete/rename notes;
- notes grouped by folder, search and Markdown preview;
- automatic light/dark/system theme;
- settings sheet with appearance, sync server/token and encryption;
- notes read/written through the Rust C ABI (`RustVaultStore`);
- seal import/create, unlock/lock and note encryption;
- attachments browser and preview;
- on-demand sync with progress and conflict counts.

Build:

```bash
cd ios
./build-rust.sh
xtool dev
```

The first run signs with your Apple ID and installs over USB. A free Apple ID
certificate expires after 7 days; rerun `xtool dev` before expiry.

## Install & build

### Desktop

To use **this fork** (with remote access, sync and the web client), build it
from source:

```bash
cargo build --release -p frankmd-app
# binary: target/release/frankmdfast
```

Requirements: Rust 1.92+ and the GPUI Linux dependencies documented by the
upstream project. The upstream desktop app is available from
[upstream Releases](https://github.com/akitaonrails/FrankMDFast/releases) and the
`frankmdfast-bin` AUR package. Those packages do **not** include this fork's
remote-access or mobile-sync changes.

### Web

There is nothing to install on the client: start the desktop server and open
the URL in a browser on the same Tailscale network.

### Android / iOS

See the platform sections above. Both clients are built from this repository
and are not published as app-store binaries.

## Architecture

Everything that touches the vault lives in Rust. The desktop, the web server,
the Android JNI bridge and the iOS C ABI are thin shells over the same core.

| Crate | Responsibility |
|---|---|
| `frankmd-core` | Domain rules, note model, config resolution, search, Hugo |
| `frankmd-fs` | Vault access, atomic writes, revisions, sync manifest |
| `frankmd-crypto` | Seal format, Argon2id, XChaCha20-Poly1305, media encryption |
| `frankmd-remote` | Axum server, auth, SSE, web client, `/api/sync/*` |
| `frankmd-mobile` | JNI bridge used by the Android app |
| `frankmd-ios` | C ABI used by the iOS app |
| `frankmd-app` | GPUI desktop shell, editor, preview, dialogs, settings |
| `frankmd-markdown`, `frankmd-net`, `frankmd-ai`, `frankmd-media`, `frankmd-theme`, `frankmd-watch`, `frankmd-vim`, `frankmd-workspace` | Markdown pipeline, HTTP policy, AI providers, media, themes, file watching, Vim engine, workspace |

The `.fed` file, environment variables and on-disk layout are compatible with
the original FrankMD, so you can switch between the two apps on the same notes
folder.

## Status and known gaps

- Desktop: feature-complete against the upstream port; the full test suite
  runs on Linux, Windows CI runs the Windows-only paths and macOS CI checks the
  build.
- Web: covered by the remote API tests; the client is the same one served by
  `frankmd-remote`.
- Android: working client with sync, search, preview, PDF share and encryption;
  conflict-resolution UI is the remaining polish.
- iOS: working client with local vault, search, preview, attachments,
  encryption and on-demand sync; background sync (`BGTaskScheduler`),
  Keychain-backed seal and attachment encryption are next.
- Known desktop gaps: relative line numbers; typed insert-mode text after `cw`
  takes a second undo; Hebrew has no right-to-left layout yet (GPUI has no
  bidi); secrets in the global config file are plain text (0600), with no
  keyring yet; on Windows a second launch opens a new window.

## Releases

| Version | Highlights |
|---|---|
| **0.1.1** | Windows note/folder creation fix, window title bar on Linux/Windows, macOS traffic-light and Dock/⌘Q fixes |
| **0.1.0** | First native Rust/GPUI release: editor, preview, Vim mode, 18 themes, AI, Hugo, media |

Download the upstream desktop builds from
[upstream Releases](https://github.com/akitaonrails/FrankMDFast/releases). This
fork's desktop binary is built from source with `cargo build --release -p
frankmd-app`; the Android and iOS apps are built from `android/` and `ios/`.

Bump `version` in `Cargo.toml`, optionally add `releases/vX.Y.Z.md` (used as the
release notes), then push the tag `vX.Y.Z`. `.github/workflows/release.yml`
builds Linux, macOS, Windows and the AUR package; signing, notarization and the
AUR push turn on when their secrets exist.

## Documentation

| Doc | What |
|---|---|
| [docs/remote.md](docs/remote.md) | Remote security model, HTTP API and systemd unit |
| [docs/encryption.md](docs/encryption.md) | Seal format, key backup, password rotation and limits |
| [docs/architecture.md](docs/architecture.md) | Crate layout, domain rules, threading, config model |
| [docs/inventory/backend.md](docs/inventory/backend.md) | Every backend feature, config key and ENV var |
| [docs/inventory/frontend.md](docs/inventory/frontend.md) | Every UI feature, shortcut, dialog and theme |
| [docs/testing/](docs/testing/) | Porting reports and the UI test plan |
| [FORK.md](FORK.md) | What this fork changes and how to report issues |

## License

MIT. The original copyright notice in [LICENSE](LICENSE) is kept unchanged.
This fork is independent from the upstream project; see [FORK.md](FORK.md).
