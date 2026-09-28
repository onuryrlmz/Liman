# ⚓ Liman

SSH, SFTP ve terminal için tek liman. MobaXterm benzeri, macOS / Windows / Ubuntu'da çalışan SSH istemcisi.
Tauri 2 (Rust) + Svelte 5 + xterm.js.

## Özellikler

- Sekmeli terminal: yerel kabuk (zsh/bash, PowerShell/cmd/WSL) ve SSH
- Oturum yöneticisi: gruplar (sürükle-bırak ile taşıma, yeniden adlandırma, gruba toplu bağlanma),
  arama, çoğaltma; parolalar sistem kasasında (Keychain / Windows Credential Manager / Secret Service)
- Oturumları dışarı / içeri aktarma: parolalar ve özel anahtarlar dahil edilirse dosyanın tamamı
  belirlenen parolayla şifrelenir (Argon2id + XChaCha20-Poly1305); tüm oturumlar ya da tek grup
- SSH kimlik doğrulama: parola, keyboard-interactive, özel anahtar (parolalı da olur),
  otomatik (`~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa`)
- Sunucu anahtarı onayı: yeni sunucuda parmak izi gösterilip onay istenir; anahtar değişmişse
  açık bir uyarıyla sorulur (`~/.ssh/known_hosts` da okunur)
- Atlama sunucusu (ProxyJump), zincir halinde de; kayıtlı bir oturum ya da `kullanıcı@sunucu:port`
- ssh-agent desteği (macOS/Linux `SSH_AUTH_SOCK`, Windows OpenSSH agent ve Pageant)
- İçe aktarma: `~/.ssh/config` ve MobaXterm (`.mxtsessions` / `MobaXterm.ini`)
- Kopan bağlantılara otomatik yeniden bağlanma
- Terminalde arama (⌘F / Ctrl+Shift+F), yakınlaştırma (⌘ + / − / 0), temalar ve yazı tipi ayarları
- Otomatik güncelleme (GitHub Releases üzerinden, imzalı)
- Bölünmüş ekran: bir sekmede yan yana / alt alta panolar; SSH panoları aynı bağlantıyı paylaşır
- MultiExec: yazılanı sekmedeki tüm panolara ya da tüm sekmelere gönderme
- Komut parçacıkları ve komut paleti (⌘P / Ctrl+Shift+P)
- Oturum kaydı: terminal çıktısı renk kodlarından arındırılmış düz metin olarak dosyaya
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
| ⌘F | Ctrl+Shift+F | Terminalde ara |
| ⌘ + / − / 0 | Ctrl + / − / 0 | Yazıyı büyüt / küçült / sıfırla |
| ⌘, | Ctrl+, | Ayarlar |
| ⌘D / ⇧⌘D | Ctrl+Shift+D / E | Sağa / aşağı böl |
| ⌘] / ⌘[ | Ctrl+Shift+] / [ | Sonraki / önceki pano |
| ⇧⌘I / ⌥⇧⌘I | Ctrl+Shift+I / Ctrl+Alt+Shift+I | MultiExec: sekme / tüm sekmeler |
| ⌘P | Ctrl+Shift+P | Komut paleti |
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

Yerel paket derlemesi güncelleme imzası ister:
`TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/liman.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" npm run tauri build`.
CI'da bu anahtar `TAURI_SIGNING_PRIVATE_KEY` secret'ında durur.

Üç platformun paketleri `.github/workflows/build.yml` ile GitHub Actions'da üretilir
(`v*` etiketi atınca taslak release oluşur).

Veriler: oturumlar `<config>/Liman/sessions.json`, host anahtarları
`<config>/Liman/known_hosts` (`LIMAN_CONFIG_DIR` ile değiştirilebilir).
Eski `yrlmzterm` klasörü ve kasadaki parolalar ilk açılışta otomatik taşınır.

---

© 2026 onuryrlmz. Tüm hakları saklıdır.
