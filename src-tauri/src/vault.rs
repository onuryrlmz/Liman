//! Ana parola. Açıkken oturumlar, gruplar ve kayıtlı parolalar düz JSON ve sistem kasası
//! yerine tek bir şifreli dosyada (vault.liman) durur; Liman açılışta parolayı sorar.
//! Şifreleme `.liman` yedekleriyle aynı: Argon2id + XChaCha20-Poly1305.

use std::{collections::HashMap, fs, path::PathBuf, sync::Mutex};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::{aead::Aead, XNonce};
use serde::{Deserialize, Serialize};

use crate::backup::{cipher, derive_key, random, Kdf};
use crate::store::{self, Session};

/// Kasa kilitliyken okunmak istendi; arayüz kilit ekranını gösterir.
pub const LOCKED: &str = "VAULT_LOCKED";
const MIN_PASSWORD: usize = 8;
const FORMAT: &str = "liman-vault";

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Data {
    pub sessions: Vec<Session>,
    pub groups: Vec<String>,
    /// Anahtarlık kimliği → parola (oturum parolaları, sudo parolaları).
    #[serde(default)]
    pub secrets: HashMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct VaultFile {
    format: String,
    version: u32,
    kdf: Kdf,
    nonce: String,
    data: String,
}

struct Open {
    key: [u8; 32],
    kdf: Kdf,
    data: Data,
}

// Açık kasalar yapılandırma klasörüne göre (testler her biri kendi klasörünü kullanır).
static OPEN: Mutex<Option<HashMap<PathBuf, Open>>> = Mutex::new(None);

fn path() -> PathBuf {
    store::config_dir().join("vault.liman")
}

pub fn enabled() -> bool {
    path().exists()
}

fn with_open<T>(f: impl FnOnce(&mut Option<&mut Open>) -> T) -> T {
    let mut g = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    f(&mut map.get_mut(&store::config_dir()))
}

pub fn unlocked() -> bool {
    with_open(|o| o.is_some())
}

fn seal(key: &[u8; 32], kdf: &Kdf, data: &Data) -> Result<()> {
    let nonce = XNonce::from(random::<24>()?);
    let plain = serde_json::to_vec(data)?;
    let enc = cipher(key)?
        .encrypt(&nonce, plain.as_slice())
        .map_err(|_| anyhow!("Şifreleme başarısız"))?;
    let file = VaultFile {
        format: FORMAT.into(),
        version: 1,
        kdf: kdf.clone(),
        nonce: B64.encode(nonce.as_slice()),
        data: B64.encode(enc),
    };
    let p = path();
    let tmp = p.with_extension("liman.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(&file)?).context("Kasa dosyası yazılamadı")?;
    fs::rename(tmp, p)?;
    Ok(())
}

fn open_file(password: &str) -> Result<Open> {
    let file: VaultFile = serde_json::from_str(&fs::read_to_string(path()).context("Kasa dosyası okunamadı")?)
        .context("Kasa dosyası bozuk")?;
    if file.format != FORMAT {
        bail!("Kasa dosyası bozuk");
    }
    let key = derive_key(password, &file.kdf)?;
    let nonce = B64.decode(&file.nonce).context("Kasa dosyası bozuk")?;
    let nonce = XNonce::try_from(nonce.as_slice()).map_err(|_| anyhow!("Kasa dosyası bozuk"))?;
    let plain = cipher(&key)?
        .decrypt(&nonce, B64.decode(&file.data).context("Kasa dosyası bozuk")?.as_slice())
        .map_err(|_| anyhow!("Ana parola yanlış"))?;
    let data = serde_json::from_slice(&plain).context("Kasa dosyası bozuk")?;
    Ok(Open { key, kdf: file.kdf, data })
}

pub fn unlock(password: &str) -> Result<()> {
    let open = open_file(password)?;
    let mut g = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(store::config_dir(), open);
    Ok(())
}

pub fn lock() {
    let mut g = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(m) = g.as_mut() {
        m.remove(&store::config_dir());
    }
}

/// Ana parolayı doğrular (dışarı aktarma gibi işlemlerden önce). Kasa kapalıysa her şey geçer.
pub fn check(password: &str) -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let kdf = with_open(|o| o.as_ref().map(|o| (o.kdf.clone(), o.key)));
    match kdf {
        Some((kdf, key)) if derive_key(password, &kdf)? == key => Ok(()),
        Some(_) => bail!("Ana parola yanlış"),
        None => open_file(password).map(|_| ()),
    }
}

pub fn read<T>(f: impl FnOnce(&Data) -> T) -> Result<T> {
    with_open(|o| o.as_ref().map(|o| f(&o.data))).ok_or_else(|| anyhow!(LOCKED))
}

/// Veriyi değiştirir ve kasayı yeniden şifreleyip yazar.
pub fn write(f: impl FnOnce(&mut Data)) -> Result<()> {
    with_open(|o| {
        let o = o.as_mut().ok_or_else(|| anyhow!(LOCKED))?;
        f(&mut o.data);
        seal(&o.key, &o.kdf, &o.data)
    })
}

/// Yeni kasa oluşturur ve açık bırakır.
pub fn create(password: &str, data: Data) -> Result<()> {
    if enabled() {
        bail!("Ana parola zaten tanımlı");
    }
    if password.chars().count() < MIN_PASSWORD {
        bail!("Ana parola en az {MIN_PASSWORD} karakter olmalı");
    }
    let kdf = Kdf::new()?;
    let key = derive_key(password, &kdf)?;
    seal(&key, &kdf, &data)?;
    let mut g = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(store::config_dir(), Open { key, kdf, data });
    Ok(())
}

/// Parola doğruysa kasanın içeriği (kasa silinmez; çağıran verileri taşıdıktan sonra `remove`).
pub fn export_all(password: &str) -> Result<Data> {
    check(password)?;
    if !unlocked() {
        unlock(password)?;
    }
    read(Data::clone)
}

pub fn remove() -> Result<()> {
    lock();
    fs::remove_file(path()).context("Kasa dosyası silinemedi")
}

pub fn change(old: &str, new: &str) -> Result<()> {
    if new.chars().count() < MIN_PASSWORD {
        bail!("Ana parola en az {MIN_PASSWORD} karakter olmalı");
    }
    let data = export_all(old)?;
    let kdf = Kdf::new()?;
    let key = derive_key(new, &kdf)?;
    seal(&key, &kdf, &data)?;
    with_open(|o| {
        if let Some(o) = o.as_mut() {
            o.key = key;
            o.kdf = kdf;
        }
    });
    Ok(())
}
