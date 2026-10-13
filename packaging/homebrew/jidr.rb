# Homebrew cask template (Phase 12b). Lives in a tap (e.g. jidr-dev/homebrew-tap)
# once the org exists; `version` and `sha256` come from the release's
# SHA256SUMS.txt. Not published: needs a notarized DMG first (D32).
cask "jidr" do
  version "0.1.0"
  sha256 "REPLACE_WITH_SHA256_FROM_SHA256SUMS_TXT"

  url "https://github.com/jidr-dev/jidr/releases/download/v#{version}/Jidr-#{version}-arm64.dmg"
  name "Jidr"
  desc "Android emulators for the Mac, rooted in one click"
  homepage "https://jidr.ly"

  depends_on arch: :arm64
  depends_on macos: ">= :ventura"

  app "Jidr.app"
  binary "#{appdir}/Jidr.app/Contents/Helpers/jidr"

  # `jidr uninstall --purge` (D31) removes devices and the private SDK; zap
  # only removes what Homebrew can see without running Jidr.
  zap trash: [
    "~/Library/Application Support/Jidr",
    "~/Library/Caches/Jidr",
  ]
end
