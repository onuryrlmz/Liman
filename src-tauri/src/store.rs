//! Kayıtlı oturumlar (JSON) ve parolalar (işletim sisteminin kasası).

use std::{fs, path::PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[cfg_attr(test, allow(dead_code))]
const SERVICE: &str = "liman";
/// Uygulamanın eski adı; ayarlar ve parolalar buradan taşınır.
const LEGACY: &str = "yrlmzterm";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    /// ~/.ssh altındaki varsayılan anahtarlar, sonra parola.
    #[default]
    Auto,
    Password,
    Key,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    /// Terminal + yan panelde SFTP.
    #[default]
    Ssh,
    /// Kabuk açmadan yalnızca dosya tarayıcısı.
    Sftp,
    Telnet,
    /// Seri port; aygıt yolu `host` alanında.
    Serial,
    /// Uzak masaüstü (uygulama içinde, noVNC).
    Vnc,
    /// Uzak masaüstü (bilgisayardaki RDP istemcisiyle).
    Rdp,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub auth: AuthKind,
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub has_secret: bool,
    #[serde(default)]
    pub kind: SessionKind,
    /// Atlama sunucusu: kayıtlı bir oturumun kimliği ya da "kullanıcı@sunucu:port".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump: Option<String>,
    /// SFTP tarayıcısının açılacağı klasör (boşsa ana dizin); "~/proje" gibi yazılabilir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_dir: Option<String>,
    /// SSH oturumunda X11 yönlendirme.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub x11: bool,
    /// Seri port oturumunda baud hızı (aygıt yolu `host` alanındadır).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baud: Option<u32>,
    /// Bağlanınca otomatik başlatılan tüneller (içeriği arayüz belirler).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tunnels: Vec<serde_json::Value>,
    /// SFTP'yi sudo ile (root olarak) aç.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sftp_sudo: bool,
    /// Özel SFTP sunucu komutu, ör. "sudo /usr/lib/openssh/sftp-server".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sftp_command: Option<String>,
}

/// Oturumun kayıtlı sudo parolasının anahtarlıktaki adı.
pub fn sudo_secret_id(session_id: &str) -> String {
    format!("{session_id}#sudo")
}

fn default_port() -> u16 {
    22
}

#[cfg(test)]
thread_local! {
    /// Testlerde her iş parçacığı kendi ayar klasörünü kullanır.
    pub static TEST_CONFIG_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

pub fn config_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = TEST_CONFIG_DIR.with(|d| d.borrow().clone()) {
        let _ = fs::create_dir_all(&dir);
        return dir;
    }
    if let Some(dir) = std::env::var_os("LIMAN_CONFIG_DIR") {
        let dir = PathBuf::from(dir);
        let _ = fs::create_dir_all(&dir);
        return dir;
    }
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("Liman");
    let legacy = base.join(LEGACY);
    if !dir.exists() && legacy.is_dir() {
        let _ = fs::rename(&legacy, &dir);
    }
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn expand_tilde(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(p),
    }
}

fn sessions_path() -> PathBuf {
    config_dir().join("sessions.json")
}

pub fn known_hosts_path() -> PathBuf {
    config_dir().join("known_hosts")
}

/// Uygulamanın known_hosts dosyasından bu sunucunun kayıtlarını siler.
pub fn forget_host_key(host: &str, port: u16) -> Result<()> {
    let path = known_hosts_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(());
    };
    let pattern = if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    };
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| {
            let hosts = l.split_whitespace().next().unwrap_or("");
            !hosts.split(',').any(|h| h == pattern)
        })
        .collect();
    fs::write(&path, kept.join("\n") + "\n")?;
    Ok(())
}

pub fn load_sessions() -> Result<Vec<Session>> {
    let path = sessions_path();
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = fs::read_to_string(&path).context("Oturum dosyası okunamadı")?;
    serde_json::from_str(&text).context("Oturum dosyası bozuk")
}

fn write_json<T: Serialize + ?Sized>(path: PathBuf, value: &T) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

// ---------- Komut parçacıkları ----------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub command: String,
    /// Gönderdikten sonra Enter'a basılsın mı?
    #[serde(default = "yes")]
    pub run: bool,
}

fn yes() -> bool {
    true
}

pub fn load_snippets() -> Vec<Snippet> {
    fs::read_to_string(config_dir().join("snippets.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_snippets(snippets: &[Snippet]) -> Result<()> {
    write_json(config_dir().join("snippets.json"), snippets)
}

// ---------- Ayarlar ----------
// Arayüz ayarları; içeriğini arayüz belirler, burada yalnızca saklanır.

pub fn load_settings() -> serde_json::Value {
    fs::read_to_string(config_dir().join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

pub fn save_settings(value: &serde_json::Value) -> Result<()> {
    write_json(config_dir().join("settings.json"), value)
}

pub(crate) fn write_sessions(sessions: &[Session]) -> Result<()> {
    write_json(sessions_path(), sessions)
}

// ---------- Gruplar ----------
//
// Bir oturumun grubu `folder` alanında durur. groups.json yalnızca sıralamayı
// ve henüz oturumu olmayan boş grupları tutar. Alt gruplar yolla yazılır: "Ankara/Web".

fn groups_path() -> PathBuf {
    config_dir().join("groups.json")
}

/// "  Ankara / Web/ " → "Ankara/Web".
pub(crate) fn clean_name(name: &str) -> Result<String> {
    let n = name
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if n.is_empty() {
        bail!("Grup adı boş olamaz");
    }
    Ok(n)
}

/// `g`, `root` grubunun kendisi ya da alt grubu mu?
pub(crate) fn in_group(g: &str, root: &str) -> bool {
    g == root || (g.starts_with(root) && g[root.len()..].starts_with('/'))
}

/// Her grubun üst grupları da listede olsun (alt gruptan önce).
fn with_parents(groups: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(groups.len());
    for g in groups {
        for (i, _) in g.match_indices('/') {
            let parent = &g[..i];
            if !out.iter().any(|x| x == parent) {
                out.push(parent.to_string());
            }
        }
        if !out.contains(&g) {
            out.push(g);
        }
    }
    out
}

pub fn load_groups() -> Result<Vec<String>> {
    let path = groups_path();
    let mut groups: Vec<String> = if path.exists() {
        serde_json::from_str(&fs::read_to_string(&path)?).unwrap_or_default()
    } else {
        vec![]
    };
    for s in load_sessions()? {
        if let Some(f) = s.folder.filter(|f| !f.is_empty()) {
            groups.push(f);
        }
    }
    Ok(with_parents(groups))
}

pub(crate) fn write_groups(groups: &[String]) -> Result<()> {
    write_json(groups_path(), groups)
}

fn ensure_group(name: &str) -> Result<()> {
    let mut groups = load_groups()?;
    if !groups.iter().any(|g| g == name) {
        groups.push(name.to_string());
        write_groups(&with_parents(groups))?;
    }
    Ok(())
}

pub fn create_group(name: &str) -> Result<()> {
    ensure_group(&clean_name(name)?)
}

/// Grubu alt gruplarıyla birlikte yeniden adlandırır ya da başka bir grubun altına taşır.
pub fn rename_group(old: &str, new: &str) -> Result<()> {
    let new = clean_name(new)?;
    if new != old && in_group(&new, old) {
        bail!("Bir grup kendi alt grubunun içine taşınamaz");
    }
    let re = |g: &str| -> String {
        if in_group(g, old) {
            format!("{new}{}", &g[old.len()..])
        } else {
            g.to_string()
        }
    };
    let mut sessions = load_sessions()?;
    for s in sessions.iter_mut() {
        if let Some(f) = s.folder.as_deref().filter(|f| in_group(f, old)) {
            s.folder = Some(re(f));
        }
    }
    let groups = load_groups()?.iter().map(|g| re(g)).collect();
    write_sessions(&sessions)?;
    write_groups(&with_parents(groups))
}

/// Grubu siler. Oturumlar kalacaksa grubun oturumları ve alt grupları bir üst gruba çıkar;
/// kalmayacaksa alt gruplarıyla birlikte hepsi silinir.
pub fn delete_group(name: &str, delete_sessions: bool) -> Result<()> {
    let mut sessions = load_sessions()?;
    let mut groups = load_groups()?;
    if delete_sessions {
        for s in sessions.iter().filter(|s| s.folder.as_deref().is_some_and(|f| in_group(f, name))) {
            delete_secret(&s.id);
        }
        sessions.retain(|s| !s.folder.as_deref().is_some_and(|f| in_group(f, name)));
        groups.retain(|g| !in_group(g, name));
    } else {
        let parent = name.rsplit_once('/').map(|(p, _)| p);
        let up = |g: &str| -> Option<String> {
            if g == name {
                return parent.map(String::from);
            }
            let rest = &g[name.len() + 1..];
            Some(match parent {
                Some(p) => format!("{p}/{rest}"),
                None => rest.to_string(),
            })
        };
        for s in sessions.iter_mut() {
            if let Some(f) = s.folder.clone().filter(|f| in_group(f, name)) {
                s.folder = up(&f);
            }
        }
        groups = groups
            .iter()
            .filter_map(|g| if in_group(g, name) { up(g) } else { Some(g.clone()) })
            .collect();
    }
    write_sessions(&sessions)?;
    write_groups(&with_parents(groups))
}

pub fn move_session(id: &str, folder: Option<String>) -> Result<()> {
    let folder = folder.as_deref().and_then(|f| clean_name(f).ok());
    let mut sessions = load_sessions()?;
    let s = sessions
        .iter_mut()
        .find(|s| s.id == id)
        .context("Oturum bulunamadı")?;
    s.folder = folder.clone();
    write_sessions(&sessions)?;
    if let Some(f) = folder {
        ensure_group(&f)?;
    }
    Ok(())
}

/// Oturumu ekler ya da günceller. `secret`: None = dokunma, Some("") = sil, Some(x) = kaydet.
pub fn save_session(mut session: Session, secret: Option<String>) -> Result<Session> {
    session.folder = session.folder.as_deref().and_then(|f| clean_name(f).ok());
    let mut sessions = load_sessions()?;
    if session.id.is_empty() {
        session.id = uuid::Uuid::new_v4().to_string();
    }
    let previous = sessions.iter().find(|s| s.id == session.id);
    session.has_secret = previous.map(|s| s.has_secret).unwrap_or(false);
    match secret {
        Some(s) if s.is_empty() => {
            delete_secret(&session.id);
            session.has_secret = false;
        }
        Some(s) => {
            set_secret(&session.id, &s)?;
            session.has_secret = true;
        }
        None => {}
    }
    match sessions.iter_mut().find(|s| s.id == session.id) {
        Some(existing) => *existing = session.clone(),
        None => sessions.push(session.clone()),
    }
    write_sessions(&sessions)?;
    if let Some(f) = session.folder.as_deref().filter(|f| !f.is_empty()) {
        ensure_group(f)?;
    }
    Ok(session)
}

pub fn delete_session(id: &str) -> Result<()> {
    let mut sessions = load_sessions()?;
    sessions.retain(|s| s.id != id);
    delete_secret(id);
    write_sessions(&sessions)
}

pub fn mark_secret_saved(id: &str) -> Result<()> {
    let mut sessions = load_sessions()?;
    if let Some(s) = sessions.iter_mut().find(|s| s.id == id) {
        s.has_secret = true;
        write_sessions(&sessions)?;
    }
    Ok(())
}

// Testlerde gerçek sistem kasası yerine bellekte tutulur.
#[cfg(test)]
pub static TEST_SECRETS: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub fn set_secret(id: &str, secret: &str) -> Result<()> {
    TEST_SECRETS
        .lock()
        .unwrap()
        .get_or_insert_with(Default::default)
        .insert(id.into(), secret.into());
    Ok(())
}

#[cfg(test)]
pub fn get_secret(id: &str) -> Option<String> {
    TEST_SECRETS.lock().unwrap().as_ref()?.get(id).cloned()
}

#[cfg(test)]
fn delete_secret(id: &str) {
    if let Some(m) = TEST_SECRETS.lock().unwrap().as_mut() {
        m.remove(id);
        m.remove(&sudo_secret_id(id));
    }
}

#[cfg(not(test))]
pub fn set_secret(id: &str, secret: &str) -> Result<()> {
    keyring::Entry::new(SERVICE, id)
        .and_then(|e| e.set_password(secret))
        .context("Parola sistem kasasına kaydedilemedi")
}

#[cfg(not(test))]
pub fn get_secret(id: &str) -> Option<String> {
    if let Ok(pw) = keyring::Entry::new(SERVICE, id).and_then(|e| e.get_password()) {
        return Some(pw);
    }
    // Eski adla kaydedilmiş parolayı yeni ada taşı.
    let old = keyring::Entry::new(LEGACY, id).ok()?;
    let pw = old.get_password().ok()?;
    if set_secret(id, &pw).is_ok() {
        let _ = old.delete_credential();
    }
    Some(pw)
}

#[cfg(not(test))]
fn delete_secret(id: &str) {
    if !id.ends_with("#sudo") {
        delete_secret(&sudo_secret_id(id));
    }
    for service in [SERVICE, LEGACY] {
        if let Ok(e) = keyring::Entry::new(service, id) {
            let _ = e.delete_credential();
        }
    }
}
