//! SSH bağlantısı: etkileşimli kabuk, SFTP ve yerel port yönlendirme.

use std::{
    collections::HashMap,
    future::Future,
    path::Path,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use russh::{
    client::{self, KeyboardInteractiveAuthResponse},
    keys::{
        self,
        agent::{client::AgentClient, AgentIdentity},
        HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate,
    },
    ChannelMsg, Disconnect,
};
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot, OnceCell},
};

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
    /// Önce bağlanılacak atlama sunucusu (ProxyJump).
    pub jump: Option<Box<ConnectParams>>,
    /// Sunucudaki grafik uygulamaları bu bilgisayarda aç.
    pub x11: bool,
    /// Kabuksuz bağlantıda SFTP de gerekmez (VNC/RDP tüneli).
    pub tunnel_only: bool,
}

pub enum SshInput {
    Data(Vec<u8>),
    Resize(u32, u32),
    Close,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TunnelKind {
    /// Bu bilgisayarda dinle → sunucunun ulaştığı adrese (ssh -L).
    Local,
    /// Sunucuda dinle → bu bilgisayarın ulaştığı adrese (ssh -R).
    Remote,
    /// Bu bilgisayarda SOCKS5 vekil sunucu; hedefi istemci seçer (ssh -D).
    Socks,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TunnelInfo {
    pub id: String,
    pub kind: TunnelKind,
    pub listen_host: String,
    pub listen_port: u16,
    /// SOCKS'ta boş.
    pub target_host: String,
    pub target_port: u16,
}

/// Uzak yönlendirmeler: sunucudaki port → yerel hedef.
pub type RemoteForwards = Arc<Mutex<HashMap<u32, (String, u16)>>>;

struct Tunnel {
    info: TunnelInfo,
    /// Yerel dinleyici (yerel ve SOCKS tünellerde).
    task: Option<tokio::task::AbortHandle>,
}

pub struct SshConn {
    handle: client::Handle<Client>,
    /// Atlama sunucusu bağlantıları (hedefe yakın olan sonda).
    jumps: Vec<client::Handle<Client>>,
    sftp: OnceCell<SftpSession>,
    tunnels: Mutex<HashMap<String, Tunnel>>,
    forwards: RemoteForwards,
    x11: Option<Arc<crate::x11::X11Ctx>>,
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

/// Bilinmeyen ya da değişmiş bir sunucu anahtarı için kullanıcıya sorulan soru.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyQuestion {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    /// Daha önce farklı bir anahtar kaydedilmiş (olası saldırı).
    pub changed: bool,
}

/// Soruyu arayüze iletir; cevap `true` ise anahtara güvenilir.
pub type AskHostKey = Arc<dyn Fn(HostKeyQuestion) -> oneshot::Receiver<bool> + Send + Sync>;

pub const HOST_KEY_REJECTED: &str = "HOST_KEY_REJECTED";

pub struct Client {
    host: String,
    port: u16,
    out: Sink,
    ask: AskHostKey,
    forwards: RemoteForwards,
    x11: Option<Arc<crate::x11::X11Ctx>>,
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
        // Uygulamanın dosyası önce gelir: kullanıcının burada onayladığı yeni anahtar,
        // ~/.ssh/known_hosts'taki eski kaydın önüne geçer.
        let app = keys::check_known_hosts_path(&self.host, self.port, &pk, &ours);
        if matches!(app, Ok(true)) {
            return Ok(true);
        }
        let user = keys::check_known_hosts(&self.host, self.port, &pk);
        if matches!(user, Ok(true)) {
            return Ok(true);
        }
        let changed = matches!(app, Err(keys::Error::KeyChanged { .. }))
            || matches!(user, Err(keys::Error::KeyChanged { .. }));

        let question = HostKeyQuestion {
            host: self.host.clone(),
            port: self.port,
            algorithm: pk.algorithm().as_str().to_string(),
            fingerprint: pk.fingerprint(HashAlg::Sha256).to_string(),
            changed,
        };
        let answer = tokio::time::timeout(Duration::from_secs(600), (self.ask)(question))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false);
        if !answer {
            bail!(HOST_KEY_REJECTED);
        }
        if changed {
            store::forget_host_key(&self.host, self.port)?;
        }
        keys::known_hosts::learn_known_hosts_path(&self.host, self.port, &pk, &ours)
            .context("known_hosts dosyasına yazılamadı")?;
        let msg = format!(
            "\x1b[33mSunucu anahtarı kaydedildi: {} {}\x1b[0m\r\n",
            pk.algorithm().as_str(),
            pk.fingerprint(HashAlg::Sha256)
        );
        (self.out)(msg.into_bytes());
        Ok(true)
    }

    /// Sunucudaki bir grafik uygulama ekrana bağlanmak istiyor.
    async fn server_channel_open_x11(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<()> {
        let Some(ctx) = self.x11.clone() else {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        };
        reply.accept().await;
        let out = self.out.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::x11::forward(&ctx, channel.into_stream()).await {
                out(format!("\r\n\x1b[33mX11: {e}\x1b[0m\r\n").into_bytes());
            }
        });
        Ok(())
    }

    /// Sunucuda açılan uzak yönlendirme portuna bağlantı geldi: yerel hedefe aktar.
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<()> {
        let target = self.forwards.lock().unwrap().get(&connected_port).cloned();
        let Some((host, port)) = target else {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        };
        match tokio::net::TcpStream::connect((host.as_str(), port)).await {
            Ok(mut sock) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut sock, &mut stream).await;
                });
            }
            Err(_) => reply.reject(russh::ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }
}

// ---------- SOCKS5 ----------

/// SOCKS5 el sıkışması (yalnızca kimliksiz CONNECT). Hedef adres ve portu döndürür.
async fn socks5_request(sock: &mut tokio::net::TcpStream) -> Result<(String, u16)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut head = [0u8; 2];
    sock.read_exact(&mut head).await?;
    if head[0] != 5 {
        bail!("SOCKS5 değil");
    }
    let mut methods = vec![0u8; head[1] as usize];
    sock.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        sock.write_all(&[5, 0xff]).await?;
        bail!("Desteklenen kimlik doğrulama yöntemi yok");
    }
    sock.write_all(&[5, 0]).await?;
    let mut req = [0u8; 4];
    sock.read_exact(&mut req).await?;
    if req[1] != 1 {
        // Yalnızca CONNECT.
        sock.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        bail!("Desteklenmeyen SOCKS komutu");
    }
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            sock.read_exact(&mut a).await?;
            std::net::Ipv4Addr::from(a).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            sock.read_exact(&mut len).await?;
            let mut name = vec![0u8; len[0] as usize];
            sock.read_exact(&mut name).await?;
            String::from_utf8(name)?
        }
        4 => {
            let mut a = [0u8; 16];
            sock.read_exact(&mut a).await?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        _ => bail!("Geçersiz adres türü"),
    };
    let mut port = [0u8; 2];
    sock.read_exact(&mut port).await?;
    Ok((host, u16::from_be_bytes(port)))
}

async fn try_key(h: &mut client::Handle<Client>, user: &str, key: keys::PrivateKey) -> Result<bool> {
    let hash = h.best_supported_rsa_hash().await?.flatten();
    let res = h
        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
        .await?;
    Ok(res.success())
}

async fn try_agent_with<S>(
    h: &mut client::Handle<Client>,
    user: &str,
    mut agent: AgentClient<S>,
) -> Result<bool>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Ok(identities) = agent.request_identities().await else {
        return Ok(false);
    };
    for id in identities {
        let AgentIdentity::PublicKey { key, .. } = id else {
            continue;
        };
        let hash = if key.algorithm().is_rsa() {
            h.best_supported_rsa_hash().await?.flatten()
        } else {
            None
        };
        if let Ok(res) = h
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
        {
            if res.success() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// ssh-agent (macOS/Linux: SSH_AUTH_SOCK, Windows: OpenSSH agent ya da Pageant).
async fn try_agent(h: &mut client::Handle<Client>, user: &str) -> Result<bool> {
    #[cfg(unix)]
    if let Ok(agent) = AgentClient::connect_env().await {
        return try_agent_with(h, user, agent).await;
    }
    #[cfg(windows)]
    {
        if let Ok(agent) = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
            if try_agent_with(h, user, agent).await? {
                return Ok(true);
            }
        }
        if let Ok(agent) = AgentClient::connect_pageant().await {
            return try_agent_with(h, user, agent).await;
        }
    }
    Ok(false)
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
            if try_agent(h, user).await? {
                return Ok(true);
            }
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

type Handle = client::Handle<Client>;

/// Bağlanır ve kimlik doğrular. Atlama sunucusu varsa önce ona bağlanıp hedefe
/// onun üzerinden tünel açar; ara bağlantılar açık kalsın diye birlikte döner.
fn open<'a>(
    p: &'a ConnectParams,
    out: &'a Sink,
    ask: &'a AskHostKey,
    forwards: &'a RemoteForwards,
    x11: &'a Option<Arc<crate::x11::X11Ctx>>,
    depth: usize,
) -> Pin<Box<dyn Future<Output = Result<(Handle, Vec<Handle>)>> + Send + 'a>> {
    Box::pin(async move {
        if depth > 5 {
            bail!("Atlama sunucusu zinciri çok uzun (döngü olabilir)");
        }
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(15)),
            keepalive_max: 3,
            inactivity_timeout: None,
            ..Default::default()
        });
        let handler = Client {
            host: p.host.clone(),
            port: p.port,
            out: out.clone(),
            ask: ask.clone(),
            // Yalnızca en sondaki (hedef) bağlantı uzak yönlendirme ve X11 alır.
            forwards: if depth == 0 { forwards.clone() } else { Arc::default() },
            x11: if depth == 0 { x11.clone() } else { None },
        };
        let timeout = |_| anyhow!("{}:{} bağlantısı zaman aşımına uğradı", p.host, p.port);
        let (mut handle, chain) = match &p.jump {
            Some(jump) => {
                let (jh, mut chain) = open(jump, out, ask, forwards, &None, depth + 1).await.map_err(|e| {
                    let msg = e.to_string();
                    if msg.contains(AUTH_FAILED) {
                        anyhow!(
                            "Atlama sunucusunda kimlik doğrulama başarısız: {}@{}",
                            jump.username,
                            jump.host
                        )
                    } else {
                        e
                    }
                })?;
                out(format!(
                    "\x1b[90m{} üzerinden {}:{} adresine geçiliyor...\x1b[0m\r\n",
                    jump.host, p.host, p.port
                )
                .into_bytes());
                let ch = jh
                    .channel_open_direct_tcpip(p.host.clone(), p.port as u32, "127.0.0.1", 0)
                    .await
                    .with_context(|| {
                        format!("{} üzerinden {}:{} adresine ulaşılamadı", jump.host, p.host, p.port)
                    })?;
                let h = tokio::time::timeout(
                    Duration::from_secs(20),
                    client::connect_stream(config, ch.into_stream(), handler),
                )
                .await
                .map_err(timeout)??;
                chain.push(jh);
                (h, chain)
            }
            None => {
                let h = tokio::time::timeout(
                    Duration::from_secs(20),
                    client::connect(config, (p.host.as_str(), p.port), handler),
                )
                .await
                .map_err(timeout)??;
                (h, Vec::new())
            }
        };
        if !authenticate(&mut handle, p).await? {
            bail!(AUTH_FAILED);
        }
        Ok((handle, chain))
    })
}

pub async fn connect(
    p: ConnectParams,
    cols: u32,
    rows: u32,
    out: Sink,
    ask: AskHostKey,
    on_exit: OnExit,
) -> Result<SshTerm> {
    let forwards = RemoteForwards::default();
    let x11 = if p.x11 && p.shell {
        match crate::x11::X11Ctx::new() {
            Ok(ctx) => Some(Arc::new(ctx)),
            Err(e) => {
                out(format!("\x1b[33mX11 yönlendirme kapalı: {e}\x1b[0m\r\n").into_bytes());
                None
            }
        }
    } else {
        None
    };
    let (handle, jumps) = open(&p, &out, &ask, &forwards, &x11, 0).await?;

    let conn = Arc::new(SshConn {
        handle,
        jumps,
        sftp: OnceCell::new(),
        tunnels: Mutex::new(HashMap::new()),
        forwards,
        x11,
    });

    if !p.shell {
        // SFTP alt sistemi yoksa bağlantı hatası olarak göster.
        if !p.tunnel_only {
            conn.sftp().await?;
        }
        return Ok(conn.watch(on_exit));
    }
    conn.open_shell(cols, rows, out, on_exit).await
}

impl SshConn {
    /// Kabuksuz (yalnızca SFTP) kullanım: bağlantı koptuğunda haber verir.
    pub fn watch(self: &Arc<Self>, on_exit: OnExit) -> SshTerm {
        let weak = Arc::downgrade(self);
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
        SshTerm {
            tx: None,
            conn: self.clone(),
        }
    }

    /// Bu bağlantı üzerinde yeni bir etkileşimli kabuk kanalı açar. Bölünmüş panolar
    /// aynı bağlantıyı paylaşır; yeniden kimlik doğrulama gerekmez.
    pub async fn open_shell(
        self: &Arc<Self>,
        cols: u32,
        rows: u32,
        out: Sink,
        on_exit: OnExit,
    ) -> Result<SshTerm> {
        let channel = self.handle.channel_open_session().await?;
        channel
            .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
            .await?;
        if let Some(x) = &self.x11 {
            channel
                .request_x11(false, false, crate::x11::AUTH_PROTO, x.fake_hex(), 0)
                .await?;
        }
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

        Ok(SshTerm {
            tx: Some(tx),
            conn: self.clone(),
        })
    }

    /// Sunucu üzerinden bir TCP bağlantısı açar (ssh -W gibi).
    pub async fn direct_tcpip(
        &self,
        host: &str,
        port: u16,
    ) -> Result<russh::ChannelStream<client::Msg>> {
        let ch = self
            .handle
            .channel_open_direct_tcpip(host.to_string(), port as u32, "127.0.0.1", 0)
            .await?;
        Ok(ch.into_stream())
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

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
            if let Some(task) = t.task {
                task.abort();
            }
        }
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
        for j in self.jumps.iter().rev() {
            let _ = j.disconnect(Disconnect::ByApplication, "", "en").await;
        }
    }

    fn add_tunnel(&self, info: TunnelInfo, task: Option<tokio::task::AbortHandle>) -> TunnelInfo {
        self.tunnels.lock().unwrap().insert(info.id.clone(), Tunnel { info: info.clone(), task });
        info
    }

    /// Yerel dinleyici açar. SOCKS'ta hedef her bağlantıda el sıkışmasıyla belirlenir.
    async fn listen_local(
        self: &Arc<Self>,
        port: u16,
        socks: bool,
        target: (String, u16),
    ) -> Result<(u16, tokio::task::AbortHandle)> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .with_context(|| format!("127.0.0.1:{port} dinlenemiyor (port kullanımda olabilir)"))?;
        let port = listener.local_addr()?.port();
        let conn = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            while let Ok((mut sock, peer)) = listener.accept().await {
                let Some(conn) = conn.upgrade() else { break };
                let target = target.clone();
                tokio::spawn(async move {
                    use tokio::io::AsyncWriteExt;
                    let (host, tport) = if socks {
                        match socks5_request(&mut sock).await {
                            Ok(t) => t,
                            Err(_) => return,
                        }
                    } else {
                        target
                    };
                    let ch = conn
                        .handle
                        .channel_open_direct_tcpip(host, tport as u32, peer.ip().to_string(), peer.port() as u32)
                        .await;
                    match ch {
                        Ok(ch) => {
                            if socks && sock.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.is_err() {
                                return;
                            }
                            let mut stream = ch.into_stream();
                            let _ = tokio::io::copy_bidirectional(&mut sock, &mut stream).await;
                        }
                        // Hedefe ulaşılamadı.
                        Err(_) if socks => {
                            let _ = sock.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
                        }
                        Err(_) => {}
                    }
                });
            }
        });
        Ok((port, task.abort_handle()))
    }

    pub async fn start_tunnel(
        self: &Arc<Self>,
        kind: TunnelKind,
        listen_host: String,
        listen_port: u16,
        target_host: String,
        target_port: u16,
    ) -> Result<TunnelInfo> {
        let id = uuid::Uuid::new_v4().to_string();
        match kind {
            TunnelKind::Local | TunnelKind::Socks => {
                let socks = kind == TunnelKind::Socks;
                let (port, task) = self
                    .listen_local(listen_port, socks, (target_host.clone(), target_port))
                    .await?;
                let info = TunnelInfo {
                    id,
                    kind,
                    listen_host: "127.0.0.1".into(),
                    listen_port: port,
                    target_host: if socks { String::new() } else { target_host },
                    target_port: if socks { 0 } else { target_port },
                };
                Ok(self.add_tunnel(info, Some(task)))
            }
            TunnelKind::Remote => {
                let bind = if listen_host.trim().is_empty() { "localhost".to_string() } else { listen_host };
                let port = self
                    .handle
                    .tcpip_forward(bind.clone(), listen_port as u32)
                    .await
                    .map_err(|e| {
                        anyhow!(
                            "Sunucu {bind}:{listen_port} yönlendirmesini reddetti ({e}). \
                             AllowTcpForwarding kapalı ya da port kullanımda olabilir."
                        )
                    })?;
                // Sunucu 0 isteğine ayırdığı portu döndürür; belirli port istenince 0 dönebilir.
                let port = if port == 0 { listen_port as u32 } else { port };
                self.forwards.lock().unwrap().insert(port, (target_host.clone(), target_port));
                let info = TunnelInfo {
                    id,
                    kind,
                    listen_host: bind,
                    listen_port: port as u16,
                    target_host,
                    target_port,
                };
                Ok(self.add_tunnel(info, None))
            }
        }
    }

    pub async fn stop_tunnel(&self, id: &str) {
        let t = self.tunnels.lock().unwrap().remove(id);
        let Some(t) = t else { return };
        if let Some(task) = t.task {
            task.abort();
        }
        if t.info.kind == TunnelKind::Remote {
            self.forwards.lock().unwrap().remove(&(t.info.listen_port as u32));
            let _ = self
                .handle
                .cancel_tcpip_forward(t.info.listen_host, t.info.listen_port as u32)
                .await;
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
