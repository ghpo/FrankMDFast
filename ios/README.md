# FrankMDFast for iOS

A native SwiftUI client for your FrankMDFast vault, compiled with
[xtool](https://github.com/xtool-org/xtool) on Linux. The app talks to the same
Rust core as the desktop and Android clients through the C ABI in
`crates/frankmd-ios`.

## Features

- SwiftUI notes drawer with a local vault in `Documents/vault`.
- Create, open, edit, save, delete and rename notes; notes grouped by folder.
- Search, Markdown preview and an attachments browser.
- Automatic light/dark/system theme.
- Settings sheet with appearance, sync server/token and encryption.
- Seal import/create, unlock/lock and note encryption (same Argon2id +
  XChaCha20-Poly1305 format as the desktop).
- On-demand sync with the desktop server: manifest diff, upload/download/delete,
  progress and conflict counts.
- Notes read/written through the Rust C ABI (`RustVaultStore`).

## Build

Prerequisites:

- Swift 6.4 Linux toolchain;
- xtool AppImage;
- Apple `Xcode.xip` downloaded with your Apple ID;
- usbmuxd and an iPhone with Developer Mode enabled.

```bash
cd ios
./build-rust.sh
xtool dev
```

The first run signs with your Apple ID and installs over USB. A free Apple ID
certificate expires after 7 days; rerun `xtool dev` before expiry.

## Rust bridge

The C ABI lives in `crates/frankmd-ios`; the header is
`Sources/CFrankMD/include/frankmd_ios.h`. `build-rust.sh` compiles the static
library for `aarch64-apple-ios` and places it in `Frameworks/` (ignored by Git).

The Swift target links `libfrankmd_ios.a` from `Frameworks/`.

## Status

Working client with local vault, editor, preview, attachments, search,
encryption and on-demand sync. Next: background sync with `BGTaskScheduler`,
Keychain-backed seal storage and attachment encryption.
