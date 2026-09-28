//! Oturumları dışarı / içeri aktarma.
//!
//! Parolalar dahilse tüm içerik kullanıcının belirlediği parolayla şifrelenir:
//! anahtar Argon2id ile türetilir, veri XChaCha20-Poly1305 ile şifrelenir.
//! Parolasız dışa aktarmada dosya düz JSON'dur ve hiçbir gizli bilgi içermez.

use std::{fs, path::Path};

use anyhow::{anyhow, bail, Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

use crate::store::{self, Session};

pub const PASSWORD_REQUIRED: &str = "PASSWORD_REQUIRED";
pub const WRONG_PASSWORD: &str = "WRONG_PASSWORD";

const FORMAT: &str = "liman-sessions";
const VERSION: u32 = 1;
const MIN_PASSWORD: usize = 8;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportedSession {
    #[serde(flatten)]
    session: Session,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
    /// Özel anahtar dosyasının içeriği (yalnızca şifreli dışa aktarmada).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_data: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Payload {
    groups: Vec<String>,
    sessions: Vec<ExportedSession>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    snippets: Vec<store::Snippet>,
}

#[derive(Serialize, Deserialize)]
struct Kdf {
    alg: String,
    m: u32,
    t: u32,
    p: u32,
    salt: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    format: String,
    version: u32,
    app_version: String,
    encrypted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kdf: Option<Kdf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload: Option<Payload>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub sessions: usize,
    pub secrets: usize,
    pub keys: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub added: usize,
    pub updated: usize,
    pub secrets: usize,
    pub keys: usize,
    pub groups: usize,
    pub snippets: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub encrypted: bool,
    /// Şifresiz dosyada oturum sayısı; şifreliyse bilinmez.
    pub sessions: Option<usize>,
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| anyhow!("Rastgele sayı üretilemedi: {e}"))?;
    Ok(b)
}

fn derive_key(password: &str, kdf: &Kdf) -> Result<[u8; 32]> {
    if kdf.alg != "argon2id" {
        bail!("Desteklenmeyen anahtar türetme: {}", kdf.alg);
    }
    let salt = B64.decode(&kdf.salt).context("Dosya bozuk (salt)")?;
    let params = Params::new(kdf.m, kdf.t, kdf.p, Some(32)).map_err(|e| anyhow!("{e}"))?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(|e| anyhow!("{e}"))?;
    Ok(key)
}

fn cipher(key: &[u8; 32]) -> Result<XChaCha20Poly1305> {
    XChaCha20Poly1305::new_from_slice(key).map_err(|_| anyhow!("Geçersiz anahtar"))
}

/// `group`: None = tüm oturumlar, Some(ad) = yalnızca o grup.
pub fn export(path: &Path, password: Option<&str>, group: Option<&str>) -> Result<ExportResult> {
    let password = password.filter(|p| !p.is_empty());
    if let Some(p) = password {
        if p.chars().count() < MIN_PASSWORD {
            bail!("Dosya parolası en az {MIN_PASSWORD} karakter olmalı");
        }
    }
    let sessions: Vec<Session> = store::load_sessions()?
        .into_iter()
        .filter(|s| group.is_none() || s.folder.as_deref() == group)
        .collect();
    if sessions.is_empty() {
        bail!("Dışarı aktarılacak oturum yok");
    }
    let groups = match group {
        Some(g) => vec![g.to_string()],
        None => store::load_groups()?,
    };

    let mut res = ExportResult {
        sessions: sessions.len(),
        secrets: 0,
        keys: 0,
    };
    let exported = sessions
        .into_iter()
        .map(|mut session| {
            let mut e = ExportedSession {
                secret: None,
                key_data: None,
                session: session.clone(),
            };
            if password.is_some() {
                if session.has_secret {
                    e.secret = store::get_secret(&session.id);
                    res.secrets += e.secret.is_some() as usize;
                }
                if session.auth == store::AuthKind::Key {
                    if let Some(p) = session.key_path.as_deref() {
                        e.key_data = fs::read_to_string(store::expand_tilde(p)).ok();
                        res.keys += e.key_data.is_some() as usize;
                    }
                }
            }
            session.has_secret = e.secret.is_some();
            e.session = session;
            e
        })
        .collect();
    let payload = Payload {
        groups,
        sessions: exported,
        // Parçacıklar yalnızca tam yedekte (grup seçilmemişken) taşınır.
        snippets: if group.is_none() { store::load_snippets() } else { vec![] },
    };

    let mut env = Envelope {
        format: FORMAT.into(),
        version: VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        encrypted: password.is_some(),
        kdf: None,
        nonce: None,
        data: None,
        payload: None,
    };
    match password {
        Some(pw) => {
            let kdf = Kdf {
                alg: "argon2id".into(),
                m: 64 * 1024,
                t: 3,
                p: 1,
                salt: B64.encode(random::<16>()?),
            };
            let key = derive_key(pw, &kdf)?;
            let nonce = XNonce::from(random::<24>()?);
            let plain = serde_json::to_vec(&payload)?;
            let data = cipher(&key)?
                .encrypt(&nonce, plain.as_slice())
                .map_err(|_| anyhow!("Şifreleme başarısız"))?;
            env.kdf = Some(kdf);
            env.nonce = Some(B64.encode(nonce.as_slice()));
            env.data = Some(B64.encode(data));
        }
        None => env.payload = Some(payload),
    }
    fs::write(path, serde_json::to_string_pretty(&env)?)
        .with_context(|| format!("{} yazılamadı", path.display()))?;
    Ok(res)
}

fn read_envelope(path: &Path) -> Result<Envelope> {
    let text = fs::read_to_string(path).with_context(|| format!("{} okunamadı", path.display()))?;
    let env: Envelope = serde_json::from_str(&text).context("Bu bir Liman oturum dosyası değil")?;
    if env.format != FORMAT {
        bail!("Bu bir Liman oturum dosyası değil");
    }
    if env.version > VERSION {
        bail!("Dosya Liman'ın daha yeni bir sürümüyle oluşturulmuş, önce uygulamayı güncelleyin");
    }
    Ok(env)
}

pub fn inspect(path: &Path) -> Result<FileInfo> {
    let env = read_envelope(path)?;
    Ok(FileInfo {
        encrypted: env.encrypted,
        sessions: env.payload.map(|p| p.sessions.len()),
    })
}

fn decrypt(env: Envelope, password: Option<&str>) -> Result<Payload> {
    if !env.encrypted {
        return env.payload.context("Dosya bozuk (içerik yok)");
    }
    let pw = password.filter(|p| !p.is_empty()).ok_or_else(|| anyhow!(PASSWORD_REQUIRED))?;
    let kdf = env.kdf.context("Dosya bozuk (kdf)")?;
    let nonce = B64.decode(env.nonce.context("Dosya bozuk (nonce)")?)?;
    let data = B64.decode(env.data.context("Dosya bozuk (veri)")?)?;
    let nonce = XNonce::try_from(nonce.as_slice()).map_err(|_| anyhow!("Dosya bozuk (nonce)"))?;
    let key = derive_key(pw, &kdf)?;
    let plain = cipher(&key)?
        .decrypt(&nonce, data.as_slice())
        .map_err(|_| anyhow!(WRONG_PASSWORD))?;
    serde_json::from_slice(&plain).context("Dosya bozuk (içerik)")
}

/// Özel anahtarı uygulamanın klasörüne yazar ve yolunu döndürür.
fn write_key(id: &str, data: &str) -> Result<String> {
    let dir = store::config_dir().join("keys");
    fs::create_dir_all(&dir)?;
    let path = dir.join(id);
    fs::write(&path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(path.to_string_lossy().into_owned())
}

/// Aynı kimlikli oturumlar güncellenir, diğerleri eklenir.
pub fn import(path: &Path, password: Option<&str>) -> Result<ImportResult> {
    let payload = decrypt(read_envelope(path)?, password)?;
    let mut sessions = store::load_sessions()?;
    let mut res = ImportResult {
        added: 0,
        updated: 0,
        secrets: 0,
        keys: 0,
        groups: 0,
        snippets: 0,
    };

    for e in payload.sessions {
        let mut s = e.session;
        if s.id.is_empty() {
            s.id = uuid::Uuid::new_v4().to_string();
        }
        s.folder = s.folder.filter(|f| !f.trim().is_empty());
        if let Some(key) = e.key_data {
            s.key_path = Some(write_key(&s.id, &key)?);
            res.keys += 1;
        }
        let old = sessions.iter().position(|x| x.id == s.id);
        match e.secret {
            Some(secret) => {
                store::set_secret(&s.id, &secret)?;
                s.has_secret = true;
                res.secrets += 1;
            }
            // Dosyada parola yoksa mevcut kayıtlı parolayı koru.
            None => s.has_secret = old.map(|i| sessions[i].has_secret).unwrap_or(false),
        }
        match old {
            Some(i) => {
                sessions[i] = s;
                res.updated += 1;
            }
            None => {
                sessions.push(s);
                res.added += 1;
            }
        }
    }
    let mut groups = store::load_groups()?;
    for g in payload
        .groups
        .into_iter()
        .chain(sessions.iter().filter_map(|s| s.folder.clone()))
    {
        if !g.trim().is_empty() && !groups.contains(&g) {
            groups.push(g);
            res.groups += 1;
        }
    }
    // Parçacıklar: aynı kimlik güncellenir, yeniler eklenir.
    if !payload.snippets.is_empty() {
        let mut snippets = store::load_snippets();
        for sn in payload.snippets {
            match snippets.iter_mut().find(|x| x.id == sn.id) {
                Some(x) => *x = sn,
                None => snippets.push(sn),
            }
            res.snippets += 1;
        }
        store::save_snippets(&snippets)?;
    }
    store::write_sessions(&sessions)?;
    store::write_groups(&groups)?;
    Ok(res)
}
