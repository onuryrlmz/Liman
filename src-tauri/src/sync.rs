//! Yerel ve uzak klasörü karşılaştırma ve seçilen farkları eşitleme.

use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Result};
use russh_sftp::{client::SftpSession, protocol::FileAttributes};
use serde::{Deserialize, Serialize};

use crate::{sftp, ssh::remote_join};

const MAX_FILES: usize = 20_000;
/// FAT/SMB gibi dosya sistemleri zamanı 2 saniye hassasiyetle saklar.
const TOLERANCE: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Info {
    size: u64,
    mtime: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    OnlyLocal,
    OnlyRemote,
    LocalNewer,
    RemoteNewer,
    /// Zaman aynı, boyut farklı.
    Different,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    /// Kökten göreli yol, "/" ile.
    pub path: String,
    pub status: Status,
    pub local_size: Option<u64>,
    pub remote_size: Option<u64>,
    pub local_mtime: Option<u64>,
    pub remote_mtime: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub diffs: Vec<Diff>,
    pub same: usize,
    /// Dosya sınırı aşıldı, karşılaştırma eksik.
    pub truncated: bool,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Upload,
    Download,
}

#[derive(Deserialize, Debug)]
pub struct Action {
    pub path: String,
    pub direction: Direction,
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn scan_local(root: &Path, rel: &str, out: &mut BTreeMap<String, Info>) -> Result<bool> {
    let dir = if rel.is_empty() { root.to_path_buf() } else { root.join(rel) };
    for e in std::fs::read_dir(&dir)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let ft = e.file_type()?;
        if ft.is_dir() {
            if !scan_local(root, &path, out)? {
                return Ok(false);
            }
        } else if ft.is_file() {
            let m = e.metadata()?;
            out.insert(path, Info { size: m.len(), mtime: m.modified().map(secs).unwrap_or(0) });
            if out.len() >= MAX_FILES {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn scan_remote<'a>(
    s: &'a SftpSession,
    root: &'a str,
    rel: String,
    out: &'a mut BTreeMap<String, Info>,
) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
    Box::pin(async move {
        let dir = if rel.is_empty() { root.to_string() } else { remote_join(root, &rel) };
        for e in s.read_dir(dir).await? {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let ft = e.file_type();
            if ft.is_dir() {
                if !scan_remote(s, root, path, out).await? {
                    return Ok(false);
                }
            } else if ft.is_file() {
                let m = e.metadata();
                out.insert(path, Info { size: m.len(), mtime: m.mtime.unwrap_or(0) as u64 });
                if out.len() >= MAX_FILES {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    })
}

fn classify(l: Option<Info>, r: Option<Info>) -> Option<Status> {
    match (l, r) {
        (Some(_), None) => Some(Status::OnlyLocal),
        (None, Some(_)) => Some(Status::OnlyRemote),
        (Some(l), Some(r)) => {
            if l.mtime.abs_diff(r.mtime) <= TOLERANCE {
                (l.size != r.size).then_some(Status::Different)
            } else if l.mtime > r.mtime {
                Some(Status::LocalNewer)
            } else {
                Some(Status::RemoteNewer)
            }
        }
        (None, None) => None,
    }
}

pub async fn compare(s: &SftpSession, local_root: &Path, remote_root: &str) -> Result<Comparison> {
    let mut local = BTreeMap::new();
    let mut remote = BTreeMap::new();
    let lroot = local_root.to_path_buf();
    let (lfull, lmap) = tokio::task::spawn_blocking(move || {
        let full = scan_local(&lroot, "", &mut local);
        (full, local)
    })
    .await?;
    let lfull = lfull?;
    let rfull = scan_remote(s, remote_root, String::new(), &mut remote).await?;

    let mut keys: Vec<&String> = lmap.keys().chain(remote.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut diffs = Vec::new();
    let mut same = 0;
    for k in keys {
        let (l, r) = (lmap.get(k).copied(), remote.get(k).copied());
        match classify(l, r) {
            Some(status) => diffs.push(Diff {
                path: k.clone(),
                status,
                local_size: l.map(|i| i.size),
                remote_size: r.map(|i| i.size),
                local_mtime: l.map(|i| i.mtime),
                remote_mtime: r.map(|i| i.mtime),
            }),
            None => same += 1,
        }
    }
    Ok(Comparison { diffs, same, truncated: !lfull || !rfull })
}

/// Göreli yolu doğrular: "..", mutlak yol ya da boş parça kabul edilmez.
fn check_rel(rel: &str) -> Result<()> {
    if rel.is_empty() || rel.starts_with('/') || rel.split(['/', '\\']).any(|p| p.is_empty() || p == ".." || p == ".") || rel.contains(':') {
        bail!("Geçersiz yol: {rel}");
    }
    Ok(())
}

async fn mkdir_p(s: &SftpSession, root: &str, rel_dir: &str) -> Result<()> {
    let mut cur = root.to_string();
    for part in rel_dir.split('/').filter(|p| !p.is_empty()) {
        cur = remote_join(&cur, part);
        if !s.try_exists(cur.clone()).await.unwrap_or(false) {
            s.create_dir(cur.clone()).await?;
        }
    }
    Ok(())
}

/// Tek dosyayı aktarır ve değiştirilme zamanını korur.
pub async fn apply_one(
    s: &SftpSession,
    local_root: &Path,
    remote_root: &str,
    a: &Action,
    rep: &mut sftp::Reporter,
) -> Result<()> {
    check_rel(&a.path)?;
    let local: PathBuf = a.path.split('/').fold(local_root.to_path_buf(), |p, part| p.join(part));
    let remote = remote_join(remote_root, &a.path);
    match a.direction {
        Direction::Upload => {
            let mtime = std::fs::metadata(&local)?.modified().map(secs)?;
            if let Some((dir, _)) = a.path.rsplit_once('/') {
                mkdir_p(s, remote_root, dir).await?;
            }
            let dir = remote.rsplit_once('/').map(|(d, _)| if d.is_empty() { "/" } else { d }).unwrap_or(".");
            sftp::upload(s, local, dir.to_string(), rep).await?;
            let attrs = FileAttributes {
                mtime: Some(mtime as u32),
                atime: Some(mtime as u32),
                ..FileAttributes::empty()
            };
            s.set_metadata(remote, attrs).await?;
        }
        Direction::Download => {
            let mtime = s.metadata(remote.clone()).await?.mtime.unwrap_or(0) as u64;
            sftp::download(s, remote, local.clone(), rep).await?;
            let f = std::fs::File::options().write(true).open(&local)?;
            f.set_modified(UNIX_EPOCH + Duration::from_secs(mtime))?;
        }
    }
    Ok(())
}
