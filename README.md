# ⚓ Liman

SSH, SFTP ve terminal için tek liman. MobaXterm benzeri, macOS / Windows / Ubuntu'da çalışan SSH istemcisi.
Tauri 2 (Rust) + Svelte 5 + xterm.js.

## Özellikler

- Sekmeli terminal: yerel kabuk (zsh/bash, PowerShell/cmd/WSL) ve SSH
- Oturum yöneticisi: klasörler, arama, çoğaltma; parolalar sistem kasasında
  (Keychain / Windows Credential Manager / Secret Service)
- SSH kimlik doğrulama: parola, keyboard-interactive, özel anahtar (parolalı da olur),
  otomatik (`~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa`)
- Host anahtarı doğrulama: `~/.ssh/known_hosts` ile uygulamanın kendi known_hosts dosyası;
  anahtar değişmişse bağlantı reddedilir
- SSH bağlanınca otomatik açılan SFTP paneli: gezinme, yükleme/indirme (klasörler dahil,
  ilerleme göstergesiyle), sürükle-bırak yükleme, yeniden adlandırma, silme, yeni klasör,
  uzak dosyayı düzenleme
- Yerel port yönlendirme (tüneller)
- Seçince kopyala, sağ tıkla yapıştır; bağlantıları tıklayarak açma
- Oturum kapanınca `R` ile yeniden bağlanma

## Kısayollar

| macOS | Windows / Linux | İş |
|---|---|---|
| ⌘T | Ctrl+Shift+T | Yeni yerel terminal |
| ⌘W | Ctrl+Shift+W | Sekmeyi kapat |
| ⌘N | Ctrl+Shift+N | Yeni SSH oturumu |
| ⌘B | Ctrl+Shift+B | Kenar çubuğunu aç/kapat |
| Ctrl+Tab | Ctrl+Tab | Sonraki sekme |
| ⌘1…9 | | Sekmeye git |

## Geliştirme

Gerekenler: Node.js, Rust. Ubuntu'da ayrıca:
`sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`

```sh
npm install
npm run tauri dev      # geliştirme
npm run tauri build    # kurulum paketi (.dmg / .msi / .deb / .AppImage)
```

Testler (SSH testleri için bir sshd gerekir, yoksa atlanır):

```sh
cd src-tauri
LIMAN_TEST_SSH="127.0.0.1:22:kullanici:/yol/ozel_anahtar" cargo test
```

Üç platformun paketleri `.github/workflows/build.yml` ile GitHub Actions'da üretilir
(`v*` etiketi atınca taslak release oluşur).

Veriler: oturumlar `<config>/Liman/sessions.json`, host anahtarları
`<config>/Liman/known_hosts` (`LIMAN_CONFIG_DIR` ile değiştirilebilir).
Eski `yrlmzterm` klasörü ve kasadaki parolalar ilk açılışta otomatik taşınır.

---

© 2026 onuryrlmz. Tüm hakları saklıdır.
