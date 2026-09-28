//! Çift panelli SFTP için yerel dosya sistemi.

use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::sftp::Entry;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    /// Mutlak, düzgünleştirilmiş yol.
    pub path: String,
    /// Kökte `None`.
    pub parent: Option<String>,
    pub entries: Vec<Entry>,
}

fn to_string(p: &Path) -> String {
    let s = p.to_string_lossy().into_owned();
    // Windows'ta canonicalize "\\?\C:\…" döndürür; kullanıcıya düz yol gösterelim.
    s.strip_prefix(r"\\?\").map(String::from).unwrap_or(s)
}

#[cfg(unix)]
fn mode(meta: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode())
}

#[cfg(not(unix))]
fn mode(_: &fs::Metadata) -> Option<u32> {
    None
}

pub fn home() -> String {
    dirs::home_dir().map(|h| to_string(&h)).unwrap_or_else(|| "/".into())
}

pub fn list(path: &str) -> Result<Listing> {
    let dir = PathBuf::from(if path.is_empty() { home() } else { path.to_string() });
    let dir = fs::canonicalize(&dir).with_context(|| format!("{} bulunamadı", dir.display()))?;
    let mut entries = Vec::new();
    for e in fs::read_dir(&dir).with_context(|| format!("{} okunamadı", dir.display()))? {
        let Ok(e) = e else { continue };
        let Ok(lmeta) = e.path().symlink_metadata() else { continue };
        let is_link = lmeta.file_type().is_symlink();
        // Bağlantının hedefine göre klasör/dosya; kırık bağlantı dosya sayılır.
        let meta = if is_link { fs::metadata(e.path()).unwrap_or(lmeta) } else { lmeta };
        entries.push(Entry {
            name: e.file_name().to_string_lossy().into_owned(),
            path: to_string(&e.path()),
            is_dir: meta.is_dir(),
            is_link,
            size: if meta.is_dir() { 0 } else { meta.len() },
            mtime: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            perms: crate::sftp::perms_string(mode(&meta)),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Listing {
        path: to_string(&dir),
        parent: dir.parent().map(to_string),
        entries,
    })
}

pub fn mkdir(parent: &str, name: &str) -> Result<()> {
    fs::create_dir(Path::new(parent).join(name)).context("Klasör oluşturulamadı")
}

pub fn rename(path: &str, new_name: &str) -> Result<()> {
    let p = Path::new(path);
    let to = p.with_file_name(new_name);
    if to.exists() {
        anyhow::bail!("{} zaten var", to.display());
    }
    fs::rename(p, to).context("Yeniden adlandırılamadı")
}

pub fn remove(path: &str) -> Result<()> {
    let p = Path::new(path);
    if p.symlink_metadata()?.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
    .context("Silinemedi")
}
