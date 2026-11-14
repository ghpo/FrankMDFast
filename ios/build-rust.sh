#!/bin/sh
# Build the Rust C ABI for the iOS device.
# Requires the Swift SDK installed by xtool and an iOS Rust target.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SDKROOT=$(find "$HOME/.config/swiftpm/swift-sdks/darwin.artifactbundle" \
  -path '*/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk' -type d | head -1)
if [ -z "$SDKROOT" ]; then
  echo "iOS SDK not found; run xtool setup/sdk install first" >&2
  exit 1
fi
cd "$ROOT"
"$HOME/.cargo/bin/rustup" target add aarch64-apple-ios
RUSTFLAGS="-C link-arg=-isysroot -C link-arg=$SDKROOT -C link-arg=-fuse-ld=lld" \
CC_aarch64_apple_ios=/usr/bin/clang \
CFLAGS_aarch64_apple_ios="--target=arm64-apple-ios17.0 -isysroot $SDKROOT" \
CARGO_TARGET_AARCH64_APPLE_IOS_LINKER=/usr/bin/clang \
  "$HOME/.cargo/bin/cargo" build -p frankmd-ios --release --target aarch64-apple-ios
mkdir -p ios/Frameworks ios/xtool/Frameworks
cp target/aarch64-apple-ios/release/libfrankmd_ios.a ios/Frameworks/
cp target/aarch64-apple-ios/release/libfrankmd_ios.a ios/xtool/Frameworks/
