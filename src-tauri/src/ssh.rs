//! SSH bağlantısı: etkileşimli kabuk, SFTP ve yerel port yönlendirme.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use russh::{
    client::{self, KeyboardInteractiveAuthResponse},
    keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
    ChannelMsg, Disconnect,
};
use russh_sftp::client::SftpSession;
use serde::Serialize;
use tokio::sync::{mpsc, OnceCell};

use crate::store;

/// Terminale giden bayt akışı.
pub type Sink = Arc<dyn Fn(Vec<u8>) + Send + Sync>;
/// Kabuk kapandığında çıkış koduyla çağrılır.
pub type OnExit = Box<dyn FnOnce(Option<u32>) + Send>;

pub const AUTH_FAILED: &str = "AUTH_FAILED";

pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: store::AuthKind,
    pub key_path: Option<String>,
    pub secret: Option<String>,
    /// false: kabuk açmadan yalnızca SFTP.
    pub shell: bool,
}

pub enum SshInput {
    Data(Vec<u8>),
    Resize(u32, u32),
    Close,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TunnelInfo {
    pub id: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
}

struct Tunnel {
    info: TunnelInfo,
    task: tokio::task::AbortHandle,
}

pub struct SshConn {
    handle: client::Handle<Client>,
    sftp: OnceCell<SftpSession>,
    tunnels: Mutex<HashMap<String, Tunnel>>,
}

pub struct SshTerm {
    /// Yalnızca SFTP bağlantılarında kabuk yoktur.
    pub tx: Option<mpsc::UnboundedSender<SshInput>>,
    pub conn: Arc<SshConn>,
}

impl SshTerm {
    pub fn send(&self, input: SshInput) -> Result<()> {
        match &self.tx {
            Some(tx) => tx.send(input).map_err(|_| anyhow!("Bağlantı kapalı")),
            None => Ok(()),
        }
    }
}

pub struct Client {
    host: String,
    port: u16,
    out: Sink,
}

impl client::Handler for Client {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool> {
        let pk = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            // Sertifikalı host anahtarları için şimdilik gömülü açık anahtarı kontrol ediyoruz.
            PublicKeyOrCertificate::Certificate(cert) => {
                keys::PublicKey::new(cert.public_key().clone(), "")
            }
        };
        let ours = store::known_hosts_path();
        let user = keys::check_known_hosts(&self.host, self.port, &pk);
        let app = keys::check_known_hosts_path(&self.host, self.port, &pk, &ours);
        match (user, app) {
            (Err(keys::Error::KeyChanged { line }), _) => bail!(
                "UYARI: {} sunucusunun anahtarı değişmiş (~/.ssh/known_hosts satır {line}). \
                 Ortadaki adam saldırısı olabilir, bağlantı reddedildi.",
                self.host
            ),
            (_, Err(keys::Error::KeyChanged { line })) => bail!(
                "UYARI: {} sunucusunun anahtarı değişmiş ({} satır {line}). Bağlantı reddedildi.",
                self.host,
                ours.display()
            ),
            (Ok(true), _) | (_, Ok(true)) => Ok(true),
            _ => {
                keys::known_hosts::learn_known_hosts_path(&self.host, self.port, &pk, &ours)
                    .context("known_hosts dosyasına yazılamadı")?;
                let msg = format!(
                    "\x1b[33mYeni sunucu anahtarı kaydedildi: {} {}\x1b[0m\r\n",
                    pk.algorithm().as_str(),
                    pk.fingerprint(HashAlg::Sha256)
                );
                (self.out)(msg.into_bytes());
                Ok(true)
            }
        }
    }
}

async fn try_key(h: &mut client::Handle<Client>, user: &str, key: keys::PrivateKey) -> Result<bool> {
    let hash = h.best_supported_rsa_hash().await?.flatten();
    let res = h
        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
        .await?;
    Ok(res.success())
}

async fn try_password(h: &mut client::Handle<Client>, user: &str, pw: &str) -> Result<bool> {
    if h.authenticate_password(user, pw).await?.success() {
        return Ok(true);
    }
    // Bazı sunucular yalnızca keyboard-interactive kabul eder.
    let mut resp = h
        .authenticate_keyboard_interactive_start(user, None::<String>)
        .await?;
    for _ in 0..5 {
        match resp {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let answers = prompts.iter().map(|_| pw.to_string()).collect();
                resp = h.authenticate_keyboard_interactive_respond(answers).await?;
            }
        }
    }
    Ok(false)
}

async fn authenticate(h: &mut client::Handle<Client>, p: &ConnectParams) -> Result<bool> {
    let user = p.username.as_str();
    match p.auth {
        store::AuthKind::Key => {
            let path = p
                .key_path
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("Anahtar dosyası seçilmedi"))?;
            let key = keys::load_secret_key(store::expand_tilde(path), p.secret.as_deref())
                .map_err(|e| anyhow!("Anahtar okunamadı: {e}"))?;
            try_key(h, user, key).await
        }
        store::AuthKind::Password => match &p.secret {
            Some(pw) => try_password(h, user, pw).await,
            None => Ok(false),
        },
        store::AuthKind::Auto => {
            if let Some(ssh_dir) = dirs::home_dir().map(|h| h.join(".ssh")) {
                for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
                    let path = ssh_dir.join(name);
                    if !path.exists() {
                        continue;
                    }
                    if let Ok(key) = keys::load_secret_key(&path, None) {
                        if try_key(h, user, key).await? {
                            return Ok(true);
                        }
                    }
                }
            }
            match &p.secret {
                Some(pw) => try_password(h, user, pw).await,
                None => Ok(false),
            }
        }
    }
}

pub async fn connect(
    p: ConnectParams,
    cols: u32,
    rows: u32,
    out: Sink,
    on_exit: OnExit,
) -> Result<SshTerm> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        inactivity_timeout: None,
        ..Default::default()
    });
    let handler = Client {
        host: p.host.clone(),
        port: p.port,
        out: out.clone(),
    };
    let mut handle = tokio::time::timeout(
        Duration::from_secs(20),
        client::connect(config, (p.host.as_str(), p.port), handler),
    )
    .await
    .map_err(|_| anyhow!("{}:{} bağlantısı zaman aşımına uğradı", p.host, p.port))??;

    if !authenticate(&mut handle, &p).await? {
        bail!(AUTH_FAILED);
    }

    let conn = Arc::new(SshConn {
        handle,
        sftp: OnceCell::new(),
        tunnels: Mutex::new(HashMap::new()),
    });

    if !p.shell {
        // SFTP alt sistemi yoksa bağlantı hatası olarak göster.
        conn.sftp().await?;
        let weak = Arc::downgrade(&conn);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                match weak.upgrade() {
                    Some(c) if !c.handle.is_closed() => continue,
                    Some(_) => break,
                    // Sekme kapatıldı; bildirecek kimse yok.
                    None => return,
                }
            }
            on_exit(None);
        });
        return Ok(SshTerm { tx: None, conn });
    }

    let channel = conn.handle.channel_open_session().await?;
    channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await?;
    channel.request_shell(false).await?;
    let (mut read, write) = channel.split();

    let (tx, mut rx) = mpsc::unbounded_channel::<SshInput>();
    tokio::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                SshInput::Data(d) => {
                    if write.data_bytes(d).await.is_err() {
                        break;
                    }
                }
                SshInput::Resize(c, r) => {
                    let _ = write.window_change(c, r, 0, 0).await;
                }
                SshInput::Close => {
                    let _ = write.close().await;
                    break;
                }
            }
        }
    });

    tokio::spawn(async move {
        let mut code = None;
        while let Some(msg) = read.wait().await {
            match msg {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    out(data.to_vec());
                }
                ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        on_exit(code);
    });

    Ok(SshTerm { tx: Some(tx), conn })
}

impl SshConn {
    pub async fn sftp(&self) -> Result<&SftpSession> {
        self.sftp
            .get_or_try_init(|| async {
                let ch = self.handle.channel_open_session().await?;
                ch.request_subsystem(true, "sftp").await?;
                let sftp = SftpSession::new(ch.into_stream())
                    .await
                    .map_err(|e| anyhow!("SFTP başlatılamadı: {e}"))?;
                Ok::<_, anyhow::Error>(sftp)
            })
            .await
    }

    pub async fn disconnect(&self) {
        for (_, t) in self.tunnels.lock().unwrap().drain() {
            t.task.abort();
        }
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
    }

    pub async fn start_tunnel(
        self: &Arc<Self>,
        local_port: u16,
        remote_host: String,
        remote_port: u16,
    ) -> Result<TunnelInfo> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", local_port))
            .await
            .with_context(|| format!("127.0.0.1:{local_port} dinlenemiyor"))?;
        let local_port = listener.local_addr()?.port();
        let conn = Arc::downgrade(self);
        let rhost = remote_host.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut sock, peer)) = listener.accept().await {
                let Some(conn) = conn.upgrade() else { break };
                let rhost = rhost.clone();
                tokio::spawn(async move {
                    let ch = conn
                        .handle
                        .channel_open_direct_tcpip(
                            rhost,
                            remote_port as u32,
                            peer.ip().to_string(),
                            peer.port() as u32,
                        )
                        .await;
                    if let Ok(ch) = ch {
                        let mut stream = ch.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut sock, &mut stream).await;
                    }
                });
            }
        });
        let info = TunnelInfo {
            id: uuid::Uuid::new_v4().to_string(),
            local_port,
            remote_host,
            remote_port,
        };
        self.tunnels.lock().unwrap().insert(
            info.id.clone(),
            Tunnel {
                info: info.clone(),
                task: task.abort_handle(),
            },
        );
        Ok(info)
    }

    pub fn stop_tunnel(&self, id: &str) {
        if let Some(t) = self.tunnels.lock().unwrap().remove(id) {
            t.task.abort();
        }
    }

    pub fn tunnels(&self) -> Vec<TunnelInfo> {
        self.tunnels
            .lock()
            .unwrap()
            .values()
            .map(|t| t.info.clone())
            .collect()
    }
}

pub fn remote_join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn local_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
