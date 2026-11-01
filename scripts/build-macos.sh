#!/bin/bash
set -euo pipefail

repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
profile="${1:-debug}"
case "$profile" in
  debug) cargo_flags=(--profile dev); swift_profile=debug ;;
  release)
    cargo_flags=(--release); swift_profile=release
    # Rust 1.96's debug-info stripping can misalign Mach-O LINKEDIT entries
    # rejected by Xcode 27's linker (rust-lang/rust#157750).
    export CARGO_PROFILE_RELEASE_STRIP=none
    ;;
  *) echo "Usage: $0 [debug|release]" >&2; exit 2 ;;
esac

cd "$repo_dir"
cargo rustc --locked --lib --features desktop --crate-type cdylib "${cargo_flags[@]}" -- \
  -C link-arg=-Wl,-install_name,@rpath/libmena.dylib
core_dir="$repo_dir/target/$profile"

generated_dir="$repo_dir/macos/Generated"
mkdir -p "$generated_dir/MenaCore" "$generated_dir/MenaFFI"
cargo run --locked --bin mena-uniffi-bindgen --features desktop-bindgen -- \
  generate --library "$core_dir/libmena.dylib" --language swift \
  --config macos/uniffi.toml --out-dir "$generated_dir"
mv "$generated_dir/MenaCore.swift" "$generated_dir/MenaCore/"
mv "$generated_dir/MenaFFI.h" "$generated_dir/MenaFFI/"
mv "$generated_dir/MenaFFI.modulemap" "$generated_dir/MenaFFI/module.modulemap"

export MENA_CORE_LIB_DIR="$core_dir"
swift build --package-path macos --configuration "$swift_profile"
swift_bin_dir="$(swift build --package-path macos --configuration "$swift_profile" --show-bin-path)"
mkdir -p "$repo_dir/target/macos"
build_dir="$(mktemp -d "$repo_dir/target/macos/.mena-build.XXXXXX")"
trap 'rm -rf "$build_dir"' EXIT
app_dir="$build_dir/Mena.app"
mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Frameworks" "$app_dir/Contents/Resources"
cp "$swift_bin_dir/Mena" "$app_dir/Contents/MacOS/Mena"
cp "$core_dir/libmena.dylib" "$app_dir/Contents/Frameworks/"
cp macos/Info.plist "$app_dir/Contents/Info.plist"
cp -R "$swift_bin_dir/MenaDesktop_Mena.bundle" "$app_dir/Contents/Resources/"
swift macos/Tools/AppIcon.swift macos/Sources/Mena/Resources/MenaLogo.png "$app_dir/Contents/Resources/Mena.icns"
install_name_tool -delete_rpath "$core_dir" "$app_dir/Contents/MacOS/Mena"
install_name_tool -add_rpath @executable_path/../Frameworks "$app_dir/Contents/MacOS/Mena"
codesign --force --sign - "$app_dir/Contents/Frameworks/libmena.dylib"
codesign --force --sign - "$app_dir"
# Replace the bundle without modifying a running app's mapped, signed binary.
destination_app="$repo_dir/target/macos/Mena.app"
if [[ -e "$destination_app" ]]; then mv "$destination_app" "$build_dir/Previous.app"; fi
mv "$app_dir" "$destination_app"
echo "Built: $destination_app"
