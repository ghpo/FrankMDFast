# FrankMDFast for iOS

Native SwiftUI client compiled with [xtool](https://github.com/xtool-org/xtool) on Linux.

## Current milestone

Phase 1 local app shell:

- SwiftUI notes drawer, local vault in `Documents/vault`;
- create/open/edit/save/delete/rename notes;
- notes grouped by folder, search, and basic Markdown preview;
- automatic light/dark/system theme;
- settings sheet with appearance and sync fields;
- notes are read/written through the Rust C ABI (`RustVaultStore`);
- seal import/unlock/lock and preview are available in the C bridge.

Sync, attachment encryption and the Keychain UI are the next milestone.

## Prerequisites

- Swift 6.4 Linux toolchain;
- xtool AppImage;
- Apple `Xcode.xip` downloaded with your Apple ID;
- usbmuxd and an iPhone with Developer Mode enabled.

## Build/install

```bash
cd ios
xtool dev
```

The first run signs with your Apple ID and installs over USB. A free Apple ID
certificate expires after 7 days; rerun `xtool dev` before expiry.

## Rust bridge

The C ABI is in `crates/frankmd-ios` and the header is
`Sources/CFrankMD/include/frankmd_ios.h`. Build it after the iOS SDK is
available:

```bash
./build-rust.sh
```

The Swift target links `libfrankmd_ios.a` from `Frameworks/`. The current
SwiftUI milestone still uses the local Swift `VaultStore`; wiring the Rust
store and Keychain comes next.

## Next phases

1. Wire SwiftUI to the Rust C ABI and Keychain-backed seal;
2. note + attachment encryption;
3. remote sync with conflict copies;
4. background `BGTaskScheduler` sync;
5. PDF/share, icons and parity polish.
