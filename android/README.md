# FrankMDFast for Android

A native Android client for your FrankMDFast vault: Kotlin + Jetpack Compose on
top of the same Rust core used by the desktop app, through a JNI bridge
(`frankmd-mobile` → `frankmd-fs`).

## Features

- Notes tree with create/open/save/delete and rename.
- Markdown editor with live preview rendered by the Rust HTML renderer in a
  WebView.
- Full-text local search across title, path and content; locked notes match by
  title.
- Automatic sync with the desktop server every 15 minutes (WorkManager,
  network-constrained) plus one sync on app open, against `/api/sync/*`.
- Seal import from the desktop, unlock/lock, and encrypt/decrypt notes together
  with their referenced attachments (Argon2id + XChaCha20-Poly1305).
- Share a note as PDF or plain text.
- Light, dark and system themes; compact top bar, overflow actions and a bottom
  Edit/Preview/Save bar.
- App-private local vault; the notes folder lives inside the app sandbox and is
  synchronized with the desktop.

## Build

```bash
cd android
./build-rust.sh
export ANDROID_HOME="$HOME/Android/Sdk"
./gradlew :app:assembleDebug
```

The APK is written to `app/build/outputs/apk/debug/app-debug.apk`.

`build-rust.sh` compiles `frankmd-mobile` for `aarch64-linux-android` and copies
the `.so` into `app/src/main/jniLibs/` (that directory is ignored by Git).

## Architecture

| Layer | Responsibility |
|---|---|
| `FrankApp.kt` / Compose UI | Screens, editor, preview, settings, dialogs |
| `VaultStore` / `NativeVault` | JNI calls into `frankmd-mobile` |
| `SyncManager` / `SyncWorker` / `SyncScheduler` | Manifest diff, upload/download/delete, 15-minute periodic sync |
| `SettingsStore` | Server URL and sync token (token in `EncryptedSharedPreferences`) |
| `PdfShare` | Renders the note HTML to PDF and shares it |

## Status

Working client with local vault, editor, preview, search, sync, encryption and
PDF share. Conflict-resolution UI is the remaining polish.
