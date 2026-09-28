//! SFTP dosya tarayıcısı ve aktarımlar.

use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    time::{Duration, Instant, UNIX_EPOCH},
};

use anyhow::{bail, Result};
use russh_sftp::client::SftpSession;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::ssh::{local_name, remote_join};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub mtime: Option<u64>,
    pub perms: String,
}

pub fn perms_string(mode: Option<u32>) -> String {
    let Some(m) = mode else { return String::new() };
    let bits = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    bits.iter()
        .map(|(b, c)| if m & b != 0 { *c } else { '-' })
        .collect()
}

pub async fn list(sftp: &SftpSession, path: &str) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    for e in sftp.read_dir(path).await? {
        let name = e.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let meta = e.metadata();
        let full = remote_join(path, &name);
        let is_link = e.file_type().is_symlink();
        let mut is_dir = e.file_type().is_dir();
        if is_link {
            // Bağlantının hedefi klasörse içine girilebilsin.
            if let Ok(target) = sftp.metadata(full.clone()).await {
                is_dir = target.file_type().is_dir();
            }
        }
        out.push(Entry {
            name,
            path: full,
            is_dir,
            is_link,
            size: meta.len(),
            mtime: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            perms: perms_string(meta.permissions),
        });
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

pub fn remove<'a>(
    sftp: &'a SftpSession,
    path: String,
    is_dir: bool,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        if !is_dir {
            sftp.remove_file(path).await?;
            return Ok(());
        }
        for e in sftp.read_dir(path.clone()).await? {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            remove(sftp, remote_join(&path, &name), e.file_type().is_dir()).await?;
        }
        sftp.remove_dir(path).await?;
        Ok(())
    })
}

pub async fn read_text(sftp: &SftpSession, path: &str) -> Result<String> {
    let meta = sftp.metadata(path).await?;
    if meta.len() > 5 * 1024 * 1024 {
        bail!("Dosya düzenlemek için çok büyük (5 MB üstü)");
    }
    let data = sftp.read(path).await?;
    String::from_utf8(data).map_err(|_| anyhow::anyhow!("Dosya metin değil (UTF-8 okunamadı)"))
}

pub async fn write_text(sftp: &SftpSession, path: &str, text: &str) -> Result<()> {
    write_bytes(sftp, path, text.as_bytes()).await
}

pub async fn write_bytes(sftp: &SftpSession, path: &str, data: &[u8]) -> Result<()> {
    let mut f = sftp.create(path).await?;
    f.write_all(data).await?;
    f.shutdown().await?;
    Ok(())
}

// ---------- Yerel editörde düzenleme ----------

const EDIT_LIMIT: u64 = 50 * 1024 * 1024;

/// Uzak dosyayı geçici bir klasöre indirir; yerel yolunu döndürür.
pub async fn fetch_for_edit(sftp: &SftpSession, remote: &str) -> Result<PathBuf> {
    let meta = sftp.metadata(remote).await?;
    if meta.file_type().is_dir() {
        bail!("Klasörler düzenlenemez");
    }
    if meta.len() > EDIT_LIMIT {
        bail!("Dosya düzenlemek için çok büyük (50 MB üstü)");
    }
    let data = sftp.read(remote).await?;
    let name = remote.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("dosya");
    let dir = crate::store::config_dir()
        .join("edit")
        .join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&dir).await?;
    let local = dir.join(name);
    tokio::fs::write(&local, data).await?;
    Ok(local)
}

/// Yerel kopya değiştikçe sunucuya yükler. Bağlantı kapanınca (ya da `stop` düşünce) durur
/// ve geçici klasörü siler. `report(Ok)` her başarılı yüklemede, `report(Err)` hatada çağrılır.
pub async fn sync_edits(
    conn: std::sync::Weak<crate::ssh::SshConn>,
    local: PathBuf,
    remote: String,
    report: impl Fn(Result<()>) + Send + 'static,
) {
    let stamp = |p: &PathBuf| std::fs::metadata(p).ok().map(|m| (m.modified().ok(), m.len()));
    let mut last = stamp(&local);
    loop {
        tokio::time::sleep(Duration::from_millis(700)).await;
        let Some(conn) = conn.upgrade().filter(|c| !c.is_closed()) else {
            break;
        };
        let now = stamp(&local);
        if now.is_none() || now == last {
            continue;
        }
        // Editör yazmayı bitirsin: boyut/zaman bir süre sabit kalana kadar bekle.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let settled = stamp(&local);
        if settled != now {
            continue;
        }
        last = settled;
        let res = async {
            let data = tokio::fs::read(&local).await?;
            let s = conn.sftp().await?;
            write_bytes(s, &remote, &data).await
        }
        .await;
        report(res);
    }
    if let Some(dir) = local.parent() {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}

// ---------- Aktarımlar ----------

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: String,
    pub name: String,
    pub done: u64,
    pub total: u64,
    pub state: &'static str,
    pub error: Option<String>,
}

pub struct Reporter {
    emit: Box<dyn Fn(Progress) + Send + Sync>,
    id: String,
    name: String,
    done: u64,
    total: u64,
    last: Instant,
}

impl Reporter {
    pub fn new(id: String, name: String, emit: impl Fn(Progress) + Send + Sync + 'static) -> Self {
        Self {
            emit: Box::new(emit),
            id,
            name,
            done: 0,
            total: 0,
            last: Instant::now() - Duration::from_secs(1),
        }
    }

    fn emit(&mut self, state: &'static str, error: Option<String>) {
        (self.emit)(Progress {
            id: self.id.clone(),
            name: self.name.clone(),
            done: self.done,
            total: self.total,
            state,
            error,
        });
    }

    fn add(&mut self, n: u64) {
        self.done += n;
        if self.last.elapsed() > Duration::from_millis(150) {
            self.last = Instant::now();
            self.emit("run", None);
        }
    }

    pub fn finish(mut self, res: &Result<()>) {
        match res {
            Ok(()) => self.emit("done", None),
            Err(e) => self.emit("error", Some(e.to_string())),
        }
    }
}

struct Job {
    remote: String,
    local: PathBuf,
    size: u64,
    dir: bool,
}

fn collect_remote<'a>(
    sftp: &'a SftpSession,
    remote: String,
    local: PathBuf,
    jobs: &'a mut Vec<Job>,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        jobs.push(Job {
            remote: remote.clone(),
            local: local.clone(),
            size: 0,
            dir: true,
        });
        for e in sftp.read_dir(remote.clone()).await? {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let r = remote_join(&remote, &name);
            let l = local.join(&name);
            let ft = e.file_type();
            if ft.is_dir() {
                collect_remote(sftp, r, l, jobs).await?;
            } else if ft.is_file() {
                jobs.push(Job {
                    remote: r,
                    local: l,
                    size: e.metadata().len(),
                    dir: false,
                });
            }
        }
        Ok(())
    })
}

fn collect_local(local: PathBuf, remote: String, jobs: &mut Vec<Job>) -> Result<()> {
    let meta = std::fs::metadata(&local)?;
    if meta.is_dir() {
        jobs.push(Job {
            remote: remote.clone(),
            local: local.clone(),
            size: 0,
            dir: true,
        });
        for e in std::fs::read_dir(&local)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            collect_local(e.path(), remote_join(&remote, &name), jobs)?;
        }
    } else if meta.is_file() {
        jobs.push(Job {
            remote,
            local,
            size: meta.len(),
            dir: false,
        });
    }
    Ok(())
}

const CHUNK: usize = 256 * 1024;

pub async fn download(
    sftp: &SftpSession,
    remote: String,
    local: PathBuf,
    rep: &mut Reporter,
) -> Result<()> {
    let meta = sftp.metadata(remote.clone()).await?;
    let mut jobs = Vec::new();
    if meta.file_type().is_dir() {
        collect_remote(sftp, remote, local, &mut jobs).await?;
    } else {
        jobs.push(Job {
            remote,
            local,
            size: meta.len(),
            dir: false,
        });
    }
    rep.total = jobs.iter().map(|j| j.size).sum();
    let mut buf = vec![0u8; CHUNK];
    for job in jobs {
        if job.dir {
            tokio::fs::create_dir_all(&job.local).await?;
            continue;
        }
        if let Some(parent) = job.local.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut src = sftp.open(job.remote).await?;
        let mut dst = tokio::fs::File::create(&job.local).await?;
        loop {
            let n = src.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await?;
            rep.add(n as u64);
        }
        dst.flush().await?;
    }
    Ok(())
}

pub async fn upload(
    sftp: &SftpSession,
    local: PathBuf,
    remote_dir: String,
    rep: &mut Reporter,
) -> Result<()> {
    let mut jobs = Vec::new();
    let remote = remote_join(&remote_dir, &local_name(&local));
    collect_local(local, remote, &mut jobs)?;
    rep.total = jobs.iter().map(|j| j.size).sum();
    let mut buf = vec![0u8; CHUNK];
    for job in jobs {
        if job.dir {
            if !sftp.try_exists(job.remote.clone()).await.unwrap_or(false) {
                sftp.create_dir(job.remote).await?;
            }
            continue;
        }
        let mut src = tokio::fs::File::open(&job.local).await?;
        let mut dst = sftp.create(job.remote).await?;
        loop {
            let n = src.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await?;
            rep.add(n as u64);
        }
        dst.shutdown().await?;
    }
    Ok(())
}
