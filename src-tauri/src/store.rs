//! Kayıtlı oturumlar (JSON) ve parolalar (işletim sisteminin kasası).

use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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

pub fn config_dir() -> PathBuf {
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

fn write_sessions(sessions: &[Session]) -> Result<()> {
    let path = sessions_path();
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(sessions)?)?;
    fs::rename(tmp, path)?;
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

pub fn set_secret(id: &str, secret: &str) -> Result<()> {
    keyring::Entry::new(SERVICE, id)
        .and_then(|e| e.set_password(secret))
        .context("Parola sistem kasasına kaydedilemedi")
}

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

fn delete_secret(id: &str) {
    for service in [SERVICE, LEGACY] {
        if let Ok(e) = keyring::Entry::new(service, id) {
            let _ = e.delete_credential();
        }
    }
}
