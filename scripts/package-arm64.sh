#!/usr/bin/env bash
set -euo pipefail

# Local packaging helper only. It does not sign, notarize, publish, or update
# a Homebrew tap. Override AIDEBOOK_TARGET for a local cross-build.
target="${AIDEBOOK_TARGET:-aarch64-apple-darwin}"
output_dir="${AIDEBOOK_PACKAGE_DIR:-dist/aidebook-${target}}"

npm run build
cargo build --manifest-path src-tauri/Cargo.toml --release --target "$target" --bin aidebook --bin aidebook-cli --bin aidebook-core
mkdir -p "$output_dir"
cp "src-tauri/target/${target}/release/aidebook" "$output_dir/aidebook"
cp "src-tauri/target/${target}/release/aidebook-cli" "$output_dir/aidebook-cli"
cp "src-tauri/target/${target}/release/aidebook-core" "$output_dir/aidebook-core"

printf 'Local arm64 package staged at %s\n' "$output_dir"
printf 'Signing, notarization, DMG assembly, and release publication remain explicit release steps.\n'
