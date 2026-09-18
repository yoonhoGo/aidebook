cask "aidebook" do
  version "0.1.0"
  sha256 "REPLACE_WITH_RELEASE_SHA256"

  # Third-party/private release template. Replace both placeholders only after
  # a public, signed, notarized release asset exists; this file publishes nothing.
  url "https://REPLACE_WITH_PUBLIC_RELEASE_HOST/Aidebook-#{version}-arm64.dmg"
  name "Aidebook"
  desc "A local notebook for an AI assistant"
  homepage "https://REPLACE_WITH_PROJECT_HOMEPAGE"

  app "Aidebook.app"
  binary "#{appdir}/Aidebook.app/Contents/MacOS/aidebook-cli", target: "aidebook"

  zap trash: [
    "~/Library/Application Support/com.yoonhogo.aidebook",
    "~/Library/Caches/com.yoonhogo.aidebook",
    "~/Library/Preferences/com.yoonhogo.aidebook.plist",
  ]
end
