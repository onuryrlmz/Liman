#!/bin/bash
# Mac paketlerini yerelde derleyip GitHub release'ini oluşturur. Windows ve Linux paketlerini
# Mac güncelleme paketini ve latest.json'u etiket üzerine GitHub Actions ekler.
#
# Kullanım: scripts/mac-release.sh <sürüm> <notlar.md>
# Gerekenler: ~/.tauri/liman.key (güncelleme imza anahtarı), gh ile oturum açılmış olması.
set -euo pipefail
V=$1; NOTES=$2
cd "$(dirname "$0")/.."
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/liman.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
npm run tauri build -- --target aarch64-apple-darwin --bundles app
npm run tauri build -- --target universal-apple-darwin --bundles app

W=$(mktemp -d /tmp/lmnrel.XXXX)
trap 'rm -rf "$W"' EXIT
mkdmg() { # $1 = .app yolu, $2 = çıktı
  codesign --verify --deep --strict "$1"
  local S; S=$(mktemp -d); ditto "$1" "$S/Liman.app"; ln -s /Applications "$S/Applications"
  # Tauri'nin kendi dmg adımı Finder otomasyonu ister; hdiutil buna gerek duymaz.
  hdiutil create -volname Liman -srcfolder "$S" -ov -format UDZO "$2" >/dev/null; rm -rf "$S"
}
A=src-tauri/target/aarch64-apple-darwin/release/bundle/macos
U=src-tauri/target/universal-apple-darwin/release/bundle/macos
grep -q "<string>$V</string>" $A/Liman.app/Contents/Info.plist
grep -q "<string>$V</string>" $U/Liman.app/Contents/Info.plist
mkdmg $A/Liman.app "$W/Liman_${V}_aarch64.dmg"
mkdmg $U/Liman.app "$W/Liman_${V}_universal.dmg"
# Güncelleme paketi (Liman_universal.app.tar.gz) ve latest.json'u yalnızca CI yükler. Burada da
# yüklenirse CI dosyayı değiştirir ama latest.json'da yerel imza kalır ve Mac'te güncelleme
# "signature verification failed" verir (0.7.2'de oldu).
gh release create "v$V" --repo onuryrlmz/Liman --target main --title "Liman $V" --notes-file "$NOTES" \
  "$W/Liman_${V}_aarch64.dmg" "$W/Liman_${V}_universal.dmg"
