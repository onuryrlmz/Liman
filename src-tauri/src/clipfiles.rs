//! Dosya listesini sistem panosuna koyar ve okur: Liman'da Ctrl/Cmd+C ile kopyalanan uzak
//! dosyalar Explorer/Finder/Nautilus'a yapıştırılabilir, oradan kopyalananlar Liman'a.
use anyhow::{anyhow, Result};
use clipboard_rs::{Clipboard, ClipboardContext};
use std::path::PathBuf;
use std::sync::Mutex;

// X11'de panodaki veriyi bu bağlamın iş parçacığı sunar; kapanırsa pano boşalır. Bu yüzden tek
// bağlam açık tutulur.
static CTX: Mutex<Option<ClipboardContext>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&ClipboardContext) -> clipboard_rs::Result<T>) -> Result<T> {
    let mut g = CTX.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(ClipboardContext::new().map_err(|e| anyhow!("Panoya erişilemedi: {e}"))?);
    }
    f(g.as_ref().unwrap()).map_err(|e| anyhow!("Pano: {e}"))
}

pub fn set_files(paths: Vec<String>) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let paths = if cfg!(target_os = "linux") {
        paths.iter().map(|p| file_uri(p)).collect()
    } else {
        paths
    };
    with(|c| c.set_files(paths))
}

/// Panodaki dosyaların yerel yolları; dosya yoksa boş liste.
pub fn get_files() -> Result<Vec<String>> {
    let list = with(|c| Ok(c.get_files().unwrap_or_default()))?;
    Ok(list
        .into_iter()
        .map(|f| match f.strip_prefix("file://") {
            Some(rest) => percent_decode(rest),
            None => f,
        })
        .collect())
}

/// Kopyalanan dosyaların indirileceği geçici klasör. Öncekiler silinir: panoda artık onlar yok.
pub fn temp_dir() -> Result<PathBuf> {
    let root = crate::store::config_dir().join("clip");
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub(crate) fn file_uri(path: &str) -> String {
    let mut s = String::from("file://");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

pub(crate) fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
