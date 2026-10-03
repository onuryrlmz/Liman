#!/bin/bash
# Mac paketlerini yerelde derleyip GitHub release'ini oluşturur. Windows ve Linux paketlerini
# ve tam latest.json'u etiket üzerine GitHub Actions ekler.
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
cp $U/Liman.app.tar.gz "$W/Liman_universal.app.tar.gz"
cp $U/Liman.app.tar.gz.sig "$W/Liman_universal.app.tar.gz.sig"
# Actions bitene kadar Mac'ler güncellemeyi görsün diye geçici latest.json.
python3 - "$W" "$V" <<'PY'
import json, sys, datetime
w, v = sys.argv[1], sys.argv[2]
e = {"signature": open(f"{w}/Liman_universal.app.tar.gz.sig").read().strip(),
     "url": f"https://github.com/onuryrlmz/Liman/releases/download/v{v}/Liman_universal.app.tar.gz"}
json.dump({"version": v, "notes": f"Liman {v}", "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
           "platforms": {k: e for k in ["darwin-aarch64", "darwin-x86_64", "darwin-aarch64-app", "darwin-x86_64-app"]}},
          open(f"{w}/latest.json", "w"), indent=2)
PY
gh release create "v$V" --repo onuryrlmz/Liman --target main --title "Liman $V" --notes-file "$NOTES" \
  "$W/Liman_${V}_aarch64.dmg" "$W/Liman_${V}_universal.dmg" "$W/Liman_universal.app.tar.gz" \
  "$W/Liman_universal.app.tar.gz.sig" "$W/latest.json"
