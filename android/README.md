# FrankMDFast Android (phase 1)

Native Android client for an app-private local vault:

- notes tree and create/open/save/delete `.md` files;
- notes read/written through the Rust core
  (`frankmd-mobile` JNI bridge → `frankmd-fs`);
- Settings with remote server URL/token;
- automatic WorkManager sync every 15 minutes (network constrained) plus one
  sync on app start, against the desktop `/api/sync/*` endpoints;
- import a desktop-exported seal, unlock/lock and encrypt/decrypt notes and
  their referenced attachments;
- full-text local search (title, path and content; locked notes match by title);
- Markdown preview through the Rust HTML renderer in a WebView;
- compact top bar, overflow actions and a bottom Edit/Preview/Save bar.

Remaining polish: launcher icon and conflict-resolution UI.

## Build

```bash
./build-rust.sh
export ANDROID_HOME="$HOME/Android/Sdk"
./gradlew :app:assembleDebug
```

APK: `app/build/outputs/apk/debug/app-debug.apk`.

## Next steps

1. Port the vault operations to the Rust core and add the JNI/UniFFI bridge.
2. Add encryption/unlock, note and attachment crypto.
3. Add the sync engine, conflict copies and WorkManager.
