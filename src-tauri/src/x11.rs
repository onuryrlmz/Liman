//! X11 yönlendirme: sunucudaki grafik uygulamalar bu bilgisayardaki X sunucusunda açılır
//! (Linux'ta hazır; macOS'ta XQuartz, Windows'ta VcXsrv/Xming gerekir).
//!
//! OpenSSH gibi sunucuya sahte bir yetki çerezi verilir; gelen her X11 bağlantısının ilk
//! paketindeki sahte çerez, yerel X sunucusunun gerçek çereziyle değiştirilir.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const AUTH_PROTO: &str = "MIT-MAGIC-COOKIE-1";

#[derive(Debug, Clone, PartialEq)]
pub enum Display {
    Unix(PathBuf),
    Tcp(String, u16),
}

pub struct X11Ctx {
    /// Sunucuya verilen sahte çerez.
    pub fake: Vec<u8>,
    /// Yerel X sunucusunun gerçek çerezi; yoksa yetkisiz bağlanılır.
    real: Option<Vec<u8>>,
    display: Display,
}

/// DISPLAY değerini çözer: ":0", "unix:0", "localhost:10.0", "/tmp/…/org.xquartz:0".
pub fn parse_display(s: &str) -> Option<Display> {
    let (host, rest) = s.rsplit_once(':')?;
    let num: u16 = rest.split('.').next()?.parse().ok()?;
    if host.starts_with('/') {
        // macOS XQuartz: soket yolu "…:0"ın kendisi.
        return Some(Display::Unix(PathBuf::from(s)));
    }
    if host.is_empty() || host == "unix" {
        if cfg!(windows) {
            return Some(Display::Tcp("127.0.0.1".into(), 6000 + num));
        }
        return Some(Display::Unix(PathBuf::from(format!("/tmp/.X11-unix/X{num}"))));
    }
    Some(Display::Tcp(host.to_string(), 6000 + num))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// `xauth list` çıktısından MIT-MAGIC-COOKIE-1 çerezini bulur.
pub fn cookie_from_xauth(output: &str) -> Option<Vec<u8>> {
    output.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let _display = it.next()?;
        (it.next()? == AUTH_PROTO).then(|| unhex(it.next()?)).flatten()
    })
}

fn real_cookie(display: &str) -> Option<Vec<u8>> {
    // GUI uygulamalarında PATH kısıtlı olabilir; XQuartz'ın yerini de dene.
    for xauth in ["xauth", "/opt/X11/bin/xauth", "/usr/bin/xauth"] {
        if let Ok(out) = std::process::Command::new(xauth).args(["list", display]).output() {
            if out.status.success() {
                return cookie_from_xauth(&String::from_utf8_lossy(&out.stdout));
            }
        }
    }
    None
}

impl X11Ctx {
    /// Yerel ekranı bulur; X sunucusu yoksa hata verir.
    pub fn new() -> Result<Self> {
        let display_env = std::env::var("DISPLAY").ok().filter(|d| !d.is_empty());
        let display_str = display_env.clone().unwrap_or_else(|| {
            // Windows'ta VcXsrv/Xming varsayılan olarak localhost:0'da dinler.
            if cfg!(windows) { "localhost:0".into() } else { ":0".into() }
        });
        let display = parse_display(&display_str).ok_or_else(|| anyhow!("Geçersiz DISPLAY: {display_str}"))?;
        if let Display::Unix(p) = &display {
            if !p.exists() {
                bail!(
                    "Yerel X sunucusu bulunamadı ({}). macOS'ta XQuartz, Windows'ta VcXsrv kurup çalıştırın.",
                    p.display()
                );
            }
        }
        let mut fake = vec![0u8; 16];
        getrandom::fill(&mut fake).map_err(|e| anyhow!("{e}"))?;
        Ok(Self { fake, real: real_cookie(&display_str), display })
    }

    pub fn fake_hex(&self) -> String {
        hex(&self.fake)
    }
}

fn pad(n: usize) -> usize {
    (4 - n % 4) % 4
}

/// X11 bağlantı kurulum paketindeki sahte çerezi gerçeğiyle değiştirir.
/// Paket eksikse `Ok(None)`; çerez eşleşmezse hata (başkası bağlanmaya çalışıyor).
pub fn rewrite_setup(buf: &[u8], fake: &[u8], real: Option<&[u8]>) -> Result<Option<Vec<u8>>> {
    if buf.len() < 12 {
        return Ok(None);
    }
    let le = match buf[0] {
        b'l' => true,
        b'B' => false,
        _ => bail!("Geçersiz X11 bayt sırası"),
    };
    let u16_at = |i: usize| {
        let b = [buf[i], buf[i + 1]];
        (if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) }) as usize
    };
    let (name_len, data_len) = (u16_at(6), u16_at(8));
    let total = 12 + name_len + pad(name_len) + data_len + pad(data_len);
    if buf.len() < total {
        return Ok(None);
    }
    let name = &buf[12..12 + name_len];
    let data_at = 12 + name_len + pad(name_len);
    let data = &buf[data_at..data_at + data_len];
    if name != AUTH_PROTO.as_bytes() || data != fake {
        bail!("X11 yetki çerezi eşleşmedi, bağlantı reddedildi");
    }
    let put = |v: usize| -> [u8; 2] {
        let v = v as u16;
        if le { v.to_le_bytes() } else { v.to_be_bytes() }
    };
    let mut out = buf[..6].to_vec();
    match real {
        Some(cookie) => {
            out.extend(put(AUTH_PROTO.len()));
            out.extend(put(cookie.len()));
            out.extend(&buf[10..12]);
            out.extend(AUTH_PROTO.as_bytes());
            out.extend(std::iter::repeat_n(0, pad(AUTH_PROTO.len())));
            out.extend(cookie);
            out.extend(std::iter::repeat_n(0, pad(cookie.len())));
        }
        None => {
            out.extend([0, 0, 0, 0]);
            out.extend(&buf[10..12]);
        }
    }
    out.extend(&buf[total..]);
    Ok(Some(out))
}

/// Sunucudan gelen bir X11 kanalını yerel X sunucusuna bağlar.
pub async fn forward<S>(ctx: &X11Ctx, mut chan: S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let first = loop {
        let n = chan.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(v) = rewrite_setup(&buf, &ctx.fake, ctx.real.as_deref())? {
            break v;
        }
        if buf.len() > 64 * 1024 {
            bail!("X11 kurulum paketi çok büyük");
        }
    };
    match &ctx.display {
        #[cfg(unix)]
        Display::Unix(path) => {
            let mut local = tokio::net::UnixStream::connect(path).await?;
            local.write_all(&first).await?;
            tokio::io::copy_bidirectional(&mut chan, &mut local).await?;
        }
        #[cfg(not(unix))]
        Display::Unix(_) => bail!("Unix soketi desteklenmiyor"),
        Display::Tcp(host, port) => {
            let mut local = tokio::net::TcpStream::connect((host.as_str(), *port)).await?;
            local.write_all(&first).await?;
            tokio::io::copy_bidirectional(&mut chan, &mut local).await?;
        }
    }
    Ok(())
}
