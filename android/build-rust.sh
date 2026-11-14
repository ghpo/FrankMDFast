#!/bin/sh
# Build the JNI library for the connected arm64 device.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
NDK="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/30.0.16248370}"
TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64"
export CC_aarch64_linux_android="$TOOLCHAIN/bin/aarch64-linux-android26-clang"
export AR_aarch64_linux_android="$TOOLCHAIN/bin/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TOOLCHAIN/bin/aarch64-linux-android26-clang"
cd "$ROOT"
"$HOME/.cargo/bin/cargo" build -p frankmd-mobile --target aarch64-linux-android --release
mkdir -p android/app/src/main/jniLibs/arm64-v8a
cp target/aarch64-linux-android/release/libfrankmd_mobile.so \
  android/app/src/main/jniLibs/arm64-v8a/libfrankmd_mobile.so
