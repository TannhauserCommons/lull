#!/bin/sh
# Installs what scripts/android-build.sh needs besides the SDK and NDK, at the versions the repo pins:
# the Rust toolchain of rust-toolchain.toml with the Android target, and the Tauri CLI of
# app/package.json, built from source. Needs rustup. CI and F-Droid's recipe (packaging/fdroid/) run it.
set -eu

cd "$(dirname "$0")/.."
rust=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)
cli=$(sed -n 's/.*"@tauri-apps\/cli": "\(.*\)".*/\1/p' app/package.json)

rustup toolchain install "$rust" --profile minimal --target aarch64-linux-android
cargo tauri --version 2>/dev/null | grep -qx "tauri-cli $cli" || cargo install tauri-cli --version "$cli" --locked
