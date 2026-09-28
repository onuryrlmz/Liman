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

pub fn load_sessions() -> Result<Vec<Session>> {
    let path = sessions_path();
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = fs::read_to_string(&path).context("Oturum dosyası okunamadı")?;
    Ok(serde_json::from_str(&text).context("Oturum dosyası bozuk")?)
}

fn write_json<T: Serialize + ?Sized>(path: PathBuf, value: &T) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub(crate) fn write_sessions(sessions: &[Session]) -> Result<()> {
    write_json(sessions_path(), sessions)
}

// ---------- Gruplar ----------
//
// Bir oturumun grubu `folder` alanında durur. groups.json yalnızca sıralamayı
// ve henüz oturumu olmayan boş grupları tutar.

fn groups_path() -> PathBuf {
    config_dir().join("groups.json")
}

fn clean_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() {
        bail!("Grup adı boş olamaz");
    }
    Ok(n.to_string())
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
            if !groups.contains(&f) {
                groups.push(f);
            }
        }
    }
    Ok(groups)
}

pub(crate) fn write_groups(groups: &[String]) -> Result<()> {
    write_json(groups_path(), groups)
}

fn ensure_group(name: &str) -> Result<()> {
    let mut groups = load_groups()?;
    if !groups.iter().any(|g| g == name) {
        groups.push(name.to_string());
        write_groups(&groups)?;
    }
    Ok(())
}

pub fn create_group(name: &str) -> Result<()> {
    ensure_group(&clean_name(name)?)
}

pub fn rename_group(old: &str, new: &str) -> Result<()> {
    let new = clean_name(new)?;
    let mut sessions = load_sessions()?;
    for s in sessions.iter_mut().filter(|s| s.folder.as_deref() == Some(old)) {
        s.folder = Some(new.clone());
    }
    let mut groups = load_groups()?;
    if groups.contains(&new) {
        groups.retain(|g| g != old);
    } else if let Some(g) = groups.iter_mut().find(|g| *g == old) {
        *g = new;
    }
    write_sessions(&sessions)?;
    write_groups(&groups)
}

/// Grubu siler; oturumları ya grupsuz bırakır ya da onlarla birlikte siler.
pub fn delete_group(name: &str, delete_sessions: bool) -> Result<()> {
    let mut sessions = load_sessions()?;
    if delete_sessions {
        for s in sessions.iter().filter(|s| s.folder.as_deref() == Some(name)) {
            delete_secret(&s.id);
        }
        sessions.retain(|s| s.folder.as_deref() != Some(name));
    } else {
        for s in sessions.iter_mut().filter(|s| s.folder.as_deref() == Some(name)) {
            s.folder = None;
        }
    }
    let mut groups = load_groups()?;
    groups.retain(|g| g != name);
    write_sessions(&sessions)?;
    write_groups(&groups)
}

pub fn move_session(id: &str, folder: Option<String>) -> Result<()> {
    let folder = folder.map(|f| f.trim().to_string()).filter(|f| !f.is_empty());
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
    for service in [SERVICE, LEGACY] {
        if let Ok(e) = keyring::Entry::new(service, id) {
            let _ = e.delete_credential();
        }
    }
}
