//! Başka araçlardan oturum içe aktarma: OpenSSH `~/.ssh/config` ve MobaXterm.

use std::{collections::HashMap, fs, path::Path};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::store::{self, AuthKind, Session, SessionKind};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub sessions: Vec<Session>,
    /// Desteklenmeyen türler (RDP, VNC, Telnet…) nedeniyle atlananlar.
    pub unsupported: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub added: usize,
    /// Aynı sunucu/kullanıcı/port zaten kayıtlı olduğu için atlananlar.
    pub skipped: usize,
}

fn new_session(name: &str, host: &str) -> Session {
    Session {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        host: host.to_string(),
        port: 22,
        username: String::new(),
        auth: AuthKind::Auto,
        key_path: None,
        folder: None,
        has_secret: false,
        kind: SessionKind::Ssh,
        jump: None,
        baud: None,
        tunnels: vec![],
    }
}

// ---------- OpenSSH ----------

pub fn default_ssh_config() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("config"))
}

#[derive(Default, Clone)]
struct HostBlock {
    patterns: Vec<String>,
    opts: HashMap<String, String>,
}

pub fn parse_ssh_config(text: &str) -> Preview {
    let mut blocks: Vec<HostBlock> = vec![HostBlock::default()];
    let mut in_match = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (k.to_lowercase(), v.trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim()),
            None => continue,
        };
        let value = value.trim_matches('"');
        match key.as_str() {
            "host" => {
                in_match = false;
                blocks.push(HostBlock {
                    patterns: value.split_whitespace().map(String::from).collect(),
                    opts: HashMap::new(),
                });
            }
            // Match blokları koşullu; içeriklerini yok say.
            "match" => in_match = true,
            _ if in_match => {}
            _ => {
                // OpenSSH'ta ilk verilen değer geçerlidir.
                blocks.last_mut().unwrap().opts.entry(key).or_insert_with(|| value.to_string());
            }
        }
    }

    let is_pattern = |p: &str| p.contains(['*', '?', '!']);
    // "Host *" gibi joker bloklar ve dosyanın başı varsayılan değer sağlar.
    let lookup = |alias: &str, own: &HostBlock, key: &str| -> Option<String> {
        if let Some(v) = own.opts.get(key) {
            return Some(v.clone());
        }
        blocks
            .iter()
            .filter(|b| b.patterns.is_empty() || b.patterns.iter().any(|p| is_pattern(p) && glob(p, alias)))
            .find_map(|b| b.opts.get(key).cloned())
    };

    let mut sessions = Vec::new();
    let mut by_alias: HashMap<String, String> = HashMap::new();
    for b in blocks.iter().filter(|b| !b.patterns.is_empty()) {
        for alias in b.patterns.iter().filter(|p| !is_pattern(p)) {
            let host = lookup(alias, b, "hostname").unwrap_or_else(|| alias.clone());
            let mut s = new_session(alias, &host);
            s.port = lookup(alias, b, "port").and_then(|p| p.parse().ok()).unwrap_or(22);
            s.username = lookup(alias, b, "user").unwrap_or_default();
            if let Some(key) = lookup(alias, b, "identityfile") {
                s.auth = AuthKind::Key;
                s.key_path = Some(key);
            }
            s.jump = lookup(alias, b, "proxyjump").filter(|j| j != "none");
            s.folder = Some("SSH config".into());
            by_alias.insert(alias.clone(), s.id.clone());
            sessions.push(s);
        }
    }
    // ProxyJump başka bir Host takma adını gösteriyorsa o oturuma bağla.
    for s in sessions.iter_mut() {
        if let Some(j) = &s.jump {
            if let Some(id) = by_alias.get(j) {
                s.jump = Some(id.clone());
            }
        }
    }
    Preview {
        sessions,
        unsupported: 0,
    }
}

/// OpenSSH tarzı basit joker eşleme (`*` ve `?`).
fn glob(pattern: &str, text: &str) -> bool {
    fn rec(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => rec(&p[1..], t) || (!t.is_empty() && rec(p, &t[1..])),
            (Some('?'), Some(_)) => rec(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => rec(&p[1..], &t[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    rec(&p, &t)
}

// ---------- MobaXterm ----------
//
// Oturumlar MobaXterm.ini ya da dışa aktarılan .mxtsessions dosyasında
// [Bookmarks], [Bookmarks_1]… bölümlerinde durur:
//   SubRep=Klasör\Alt klasör
//   Ad= #109#0%sunucu%22%kullanıcı%...
// 109 = SSH, 140 = SFTP. Diğer türler (RDP, VNC, Telnet…) atlanır.

pub fn parse_mobaxterm(text: &str) -> Preview {
    let mut sessions = Vec::new();
    let mut unsupported = 0;
    let mut in_bookmarks = false;
    let mut folder: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.starts_with('[') {
            in_bookmarks = line.to_lowercase().starts_with("[bookmarks");
            folder = None;
            continue;
        }
        if !in_bookmarks {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        match name {
            "SubRep" => {
                let f = value.trim().replace('\\', " / ");
                folder = (!f.is_empty()).then_some(f);
                continue;
            }
            "ImgNum" => continue,
            _ => {}
        }
        let parts: Vec<&str> = value.trim().split('#').collect();
        let (Some(kind), Some(data)) = (parts.get(1), parts.get(2)) else {
            continue;
        };
        let kind = match *kind {
            "109" => SessionKind::Ssh,
            "140" => SessionKind::Sftp,
            _ => {
                unsupported += 1;
                continue;
            }
        };
        let f: Vec<&str> = data.split('%').collect();
        let host = f.get(1).copied().unwrap_or("").trim();
        if host.is_empty() {
            continue;
        }
        let mut s = new_session(name.trim(), host);
        s.port = f.get(2).and_then(|p| p.trim().parse().ok()).unwrap_or(22);
        s.username = f.get(3).copied().unwrap_or("").trim().to_string();
        s.kind = kind;
        s.folder = folder.clone().or_else(|| Some("MobaXterm".into()));
        sessions.push(s);
    }
    Preview {
        sessions,
        unsupported,
    }
}

// ---------- Ortak ----------

fn read_lossy(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("{} okunamadı", path.display()))?;
    Ok(match String::from_utf8(bytes) {
        Ok(s) => s,
        // UTF-8 değilse (eski MobaXterm sürümleri) Latin-1 olarak oku; ASCII alanlar bozulmaz.
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    })
}

pub fn preview(source: &str, path: Option<&Path>) -> Result<Preview> {
    match source {
        "ssh-config" => {
            let path = match path {
                Some(p) => p.to_path_buf(),
                None => default_ssh_config().context("Ev klasörü bulunamadı")?,
            };
            if !path.exists() {
                anyhow::bail!("{} bulunamadı", path.display());
            }
            Ok(parse_ssh_config(&read_lossy(&path)?))
        }
        "mobaxterm" => {
            let path = path.context("Dosya seçilmedi")?;
            Ok(parse_mobaxterm(&read_lossy(path)?))
        }
        other => anyhow::bail!("Bilinmeyen kaynak: {other}"),
    }
}

/// Önizlemede seçilen oturumları kaydeder; aynı sunucu/port/kullanıcı zaten varsa atlar.
pub fn import(sessions: Vec<Session>) -> Result<Imported> {
    let existing = store::load_sessions()?;
    let key = |s: &Session| (s.host.to_lowercase(), s.port, s.username.clone(), s.kind);
    let mut known: Vec<(_, String)> = existing.iter().map(|s| (key(s), s.id.clone())).collect();
    let mut res = Imported { added: 0, skipped: 0 };
    // Atlanan bir oturumu atlama sunucusu olarak gösterenler kayıtlı kopyasına bağlanır.
    let mut renamed: HashMap<String, String> = HashMap::new();
    let mut to_add = Vec::new();
    for s in sessions {
        if let Some((_, id)) = known.iter().find(|(k, _)| *k == key(&s)) {
            renamed.insert(s.id.clone(), id.clone());
            res.skipped += 1;
            continue;
        }
        known.push((key(&s), s.id.clone()));
        to_add.push(s);
    }
    for mut s in to_add {
        if let Some(j) = s.jump.as_ref().and_then(|j| renamed.get(j)) {
            s.jump = Some(j.clone());
        }
        store::save_session(s, None)?;
        res.added += 1;
    }
    Ok(res)
}
