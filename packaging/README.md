# Local packaging boundary

`../scripts/package-arm64.sh` stages local `aidebook`, `aidebook-cli`, and
`aidebook-core` binaries for `aarch64-apple-darwin`. It deliberately does not
sign, notarize, assemble a DMG, upload an asset, or push a tap.

`homebrew/Casks/aidebook.rb` is a placeholder Cask template. The URL, SHA-256,
homepage, Developer ID signature, notarization ticket, and public ownership
must be verified per release before a user-owned third-party tap is updated.
