//! Uçtan uca testler. SSH testleri gerçek bir sshd ister:
//!
//! LIMAN_TEST_SSH="127.0.0.1:2222:kullanici:/yol/ozel_anahtar" cargo test
//!
//! Değişken tanımlı değilse SSH testleri atlanır.
//!
//! Testler paralel bağlandığı için sshd_config'te `MaxStartups 100` olmalı; OpenSSH'ın
//! varsayılanı (10) aynı anda gelen bağlantıların bazılarını rastgele düşürür. OpenSSH 9.8+
//! için `PerSourcePenalties no` da gerekir: anahtarı bilerek reddeden test, kimlik
//! doğrulamadan kopan bağlantı sayıldığından kaynak adresi geçici olarak engellenir.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::oneshot,
};

use crate::{local, sftp, ssh, store};

struct Output(Arc<Mutex<Vec<u8>>>);

impl Output {
    fn new() -> (Self, ssh::Sink) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let b = buf.clone();
        (Self(buf), Arc::new(move |d| b.lock().unwrap().extend(d)))
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }

    async fn wait_for(&self, needle: &str) {
        for _ in 0..100 {
            if self.text().contains(needle) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("'{needle}' çıktıda görünmedi:\n{}", self.text());
    }
}

fn exit_signal() -> (ssh::OnExit, oneshot::Receiver<Option<u32>>) {
    let (tx, rx) = oneshot::channel();
    (
        Box::new(move |code| {
            let _ = tx.send(code);
        }),
        rx,
    )
}

/// Sunucu anahtarı sorularını kaydeder ve verilen cevabı döner.
fn asker(answer: bool) -> (ssh::AskHostKey, Arc<Mutex<Vec<ssh::HostKeyQuestion>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let a = asked.clone();
    let ask: ssh::AskHostKey = Arc::new(move |q| {
        a.lock().unwrap().push(q);
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(answer);
        rx
    });
    (ask, asked)
}

fn accept_all() -> ssh::AskHostKey {
    asker(true).0
}

fn test_params(secret: Option<&str>) -> Option<ssh::ConnectParams> {
    let spec = std::env::var("LIMAN_TEST_SSH").ok()?;
    let mut it = spec.splitn(4, ':');
    let host = it.next()?.to_string();
    let port = it.next()?.parse().ok()?;
    let username = it.next()?.to_string();
    let key = it.next()?.to_string();
    std::env::set_var(
        "LIMAN_CONFIG_DIR",
        std::env::temp_dir().join("liman-test"),
    );
    Some(ssh::ConnectParams {
        host,
        port,
        username,
        auth: store::AuthKind::Key,
        key_path: Some(key),
        secret: secret.map(String::from),
        shell: true,
        jump: None,
        x11: false,
        tunnel_only: false,
    })
}

#[test]
fn local_shell_runs() {
    let (out, sink) = Output::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
    let mut term = local::spawn(
        Some(shell.into()),
        80,
        24,
        sink,
        Box::new(move |code| {
            let _ = tx.send(code);
        }),
    )
    .unwrap();
    let (cmd, expect): (&[u8], &str) = if cfg!(windows) {
        (b"echo liman-%COMPUTERNAME:~0,0%42\r", "liman-42")
    } else {
        (b"echo liman-$((40+2))\r", "liman-42")
    };
    // Windows ConPTY açılışta imleç konumunu sorar (ESC[6n) ve yanıt gelene kadar girdiyi
    // işlemez. Uygulamada bunu xterm.js yanıtlar; testte biz yanıtlıyoruz.
    if cfg!(windows) {
        for _ in 0..50 {
            if out.text().contains("\x1b[6n") {
                term.write(b"\x1b[1;1R").unwrap();
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    term.write(cmd).unwrap();
    term.resize(120, 30).unwrap();
    term.write(b"exit 3\r").unwrap();
    let code = rx.recv_timeout(Duration::from_secs(10)).expect("kabuk kapanmadı");
    assert_eq!(code, Some(3));
    assert!(out.text().contains(expect), "{}", out.text());
}

#[tokio::test]
async fn ssh_shell_sftp_and_tunnel() {
    let Some(params) = test_params(None) else {
        eprintln!("LIMAN_TEST_SSH yok, atlandı");
        return;
    };

    // --- Kabuk ---
    let (out, sink) = Output::new();
    let (on_exit, exited) = exit_signal();
    let term = ssh::connect(params, 100, 30, sink, accept_all(), on_exit).await.unwrap();
    term
        .send(ssh::SshInput::Data(b"echo liman-$((40+2))\n".to_vec()))
        .unwrap();
    out.wait_for("liman-42").await;
    term.send(ssh::SshInput::Resize(132, 40)).unwrap();
    term
        .send(ssh::SshInput::Data(b"stty size\n".to_vec()))
        .unwrap();
    out.wait_for("40 132").await;

    // --- SFTP ---
    let s = term.conn.sftp().await.unwrap();
    let home = s.canonicalize(".").await.unwrap();
    let dir = ssh::remote_join(&home, &format!("liman-test-{}", std::process::id()));
    s.create_dir(dir.clone()).await.unwrap();

    let file = ssh::remote_join(&dir, "merhaba.txt");
    sftp::write_text(s, &file, "şğü merhaba\n").await.unwrap();
    assert_eq!(sftp::read_text(s, &file).await.unwrap(), "şğü merhaba\n");

    let listed = sftp::list(s, &dir).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "merhaba.txt");
    assert_eq!(listed[0].size, "şğü merhaba\n".len() as u64);

    // Klasör yükleme / indirme (iç içe, 1 MB'lık dosyayla).
    let local_root = std::env::temp_dir().join(format!("liman-local-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&local_root);
    let src = local_root.join("kaynak");
    std::fs::create_dir_all(src.join("alt/bos")).unwrap();
    let big: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(src.join("alt/buyuk.bin"), &big).unwrap();
    std::fs::write(src.join("kucuk.txt"), b"x").unwrap();

    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let mut rep = sftp::Reporter::new("t1".into(), "kaynak".into(), move |p| {
        ev.lock().unwrap().push(p)
    });
    let res = sftp::upload(s, src.clone(), dir.clone(), &mut rep).await;
    res.as_ref().unwrap();
    rep.finish(&res);
    let last = events.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.state, "done");
    assert_eq!(last.done, 1_000_001);

    let dst = local_root.join("indirilen");
    let mut rep = sftp::Reporter::new("t2".into(), "kaynak".into(), |_| {});
    sftp::download(s, ssh::remote_join(&dir, "kaynak"), dst.clone(), &mut rep)
        .await
        .unwrap();
    assert_eq!(std::fs::read(dst.join("alt/buyuk.bin")).unwrap(), big);
    assert_eq!(std::fs::read(dst.join("kucuk.txt")).unwrap(), b"x");
    assert!(dst.join("alt/bos").is_dir());

    let renamed = ssh::remote_join(&dir, "yeni.txt");
    s.rename(file.clone(), renamed.clone()).await.unwrap();
    assert!(s.try_exists(renamed).await.unwrap());

    sftp::remove(s, dir.clone(), true).await.unwrap();
    assert!(!s.try_exists(dir).await.unwrap());
    let _ = std::fs::remove_dir_all(&local_root);

    // --- Tünel: yerel yankı sunucusuna sunucu üzerinden ulaş ---
    let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = echo.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = sock.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    let info = term
        .conn
        .start_tunnel(ssh::TunnelKind::Local, String::new(), 0, "127.0.0.1".into(), echo_port)
        .await
        .unwrap();
    assert_eq!(term.conn.tunnels().len(), 1);
    let mut c = tokio::net::TcpStream::connect(("127.0.0.1", info.listen_port))
        .await
        .unwrap();
    c.write_all(b"tunel-ok").await.unwrap();
    let mut buf = [0u8; 8];
    tokio::time::timeout(Duration::from_secs(5), c.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"tunel-ok");
    term.conn.stop_tunnel(&info.id).await;
    assert!(term.conn.tunnels().is_empty());

    // --- Çıkış ---
    term
        .send(ssh::SshInput::Data(b"exit 3\n".to_vec()))
        .unwrap();
    let code = tokio::time::timeout(Duration::from_secs(10), exited)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(code, Some(3));
    term.conn.disconnect().await;
}

#[tokio::test]
async fn ssh_wrong_key_reports_auth_failed() {
    let Some(mut params) = test_params(None) else {
        return;
    };
    // Sunucuda yetkili olmayan yeni bir anahtar üret.
    let path = std::env::temp_dir().join(format!("liman-badkey-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let ok = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&path)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    params.key_path = Some(path.to_string_lossy().into_owned());
    let (_out, sink) = Output::new();
    let (on_exit, _) = exit_signal();
    let err = ssh::connect(params, 80, 24, sink, accept_all(), on_exit)
        .await
        .err()
        .expect("yanlış anahtarla bağlanmamalıydı");
    assert!(err.to_string().contains(ssh::AUTH_FAILED), "{err}");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("pub"));
}

// ---------- Gruplar ve dışarı/içeri aktarma ----------

fn fresh_config(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("liman-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    store::TEST_CONFIG_DIR.with(|d| *d.borrow_mut() = Some(dir.clone()));
    dir
}

fn session(name: &str, folder: Option<&str>, auth: store::AuthKind) -> store::Session {
    store::Session {
        id: String::new(),
        name: name.into(),
        host: format!("{name}.example.com"),
        port: 22,
        username: "root".into(),
        auth,
        key_path: None,
        folder: folder.map(String::from),
        has_secret: false,
        kind: store::SessionKind::Ssh,
        jump: None,
        x11: false,
        baud: None,
        tunnels: vec![],
    }
}

#[test]
fn groups_create_rename_move_delete() {
    let dir = fresh_config("groups");
    store::create_group("Boş grup").unwrap();
    let a = store::save_session(session("a", Some("Üretim"), store::AuthKind::Password), None).unwrap();
    let b = store::save_session(session("b", None, store::AuthKind::Auto), None).unwrap();
    assert_eq!(store::load_groups().unwrap(), vec!["Boş grup", "Üretim"]);

    store::move_session(&b.id, Some("Test".into())).unwrap();
    store::rename_group("Üretim", "Prod").unwrap();
    let find = |id: &str| store::load_sessions().unwrap().into_iter().find(|s| s.id == id).unwrap();
    assert_eq!(find(&a.id).folder.as_deref(), Some("Prod"));
    assert_eq!(store::load_groups().unwrap(), vec!["Boş grup", "Prod", "Test"]);

    // Var olan bir grubun adına yeniden adlandırma iki grubu birleştirir.
    store::rename_group("Test", "Prod").unwrap();
    assert_eq!(store::load_groups().unwrap(), vec!["Boş grup", "Prod"]);

    store::delete_group("Prod", false).unwrap();
    assert!(find(&a.id).folder.is_none() && find(&b.id).folder.is_none());
    store::move_session(&a.id, Some("Sil".into())).unwrap();
    store::delete_group("Sil", true).unwrap();
    assert_eq!(store::load_sessions().unwrap().len(), 1);
    assert!(store::create_group("   ").is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn export_import_roundtrip_with_secrets() {
    use crate::backup;

    // --- Kaynak makine ---
    let src = fresh_config("export");
    let key_file = src.join("id_test");
    std::fs::write(&key_file, "-----BEGIN OPENSSH PRIVATE KEY-----\ntest\n").unwrap();
    let mut keyed = session("anahtarli", Some("Üretim"), store::AuthKind::Key);
    keyed.key_path = Some(key_file.to_string_lossy().into());
    let keyed = store::save_session(keyed, Some("anahtar-parolasi".into())).unwrap();
    let pw = store::save_session(session("parolali", Some("Üretim"), store::AuthKind::Password), Some("çok gizli ş".into())).unwrap();
    let plain = store::save_session(session("parolasiz", None, store::AuthKind::Auto), None).unwrap();
    store::create_group("Boş").unwrap();

    store::save_snippets(&[store::Snippet { id: "s1".into(), name: "disk".into(), command: "df -h".into(), run: true }]).unwrap();
    let enc = src.join("yedek.liman");
    let r = backup::export(&enc, Some("dosya-parolasi"), None).unwrap();
    assert_eq!((r.sessions, r.secrets, r.keys), (3, 2, 1));
    let raw = std::fs::read_to_string(&enc).unwrap();
    for leak in ["çok gizli", "anahtar-parolasi", "BEGIN OPENSSH", "parolali", "example.com"] {
        assert!(!raw.contains(leak), "şifreli dosyada düz metin bulundu: {leak}");
    }
    assert!(backup::export(&src.join("x"), Some("kisa"), None).is_err());

    let open_file = src.join("acik.json");
    let r = backup::export(&open_file, None, Some("Üretim")).unwrap();
    assert_eq!((r.sessions, r.secrets, r.keys), (2, 0, 0));
    let raw = std::fs::read_to_string(&open_file).unwrap();
    assert!(!raw.contains("çok gizli") && !raw.contains("BEGIN OPENSSH"));
    assert!(!raw.contains("parolasiz"), "grup filtresi uygulanmadı");

    // --- Hedef makine: boş ayarlar, boş kasa ---
    *store::TEST_SECRETS.lock().unwrap() = None;
    let dst = fresh_config("import");
    let info = backup::inspect(&enc).unwrap();
    assert!(info.encrypted && info.sessions.is_none());
    let e = backup::import(&enc, None).err().unwrap();
    assert!(e.to_string().contains(backup::PASSWORD_REQUIRED));
    let e = backup::import(&enc, Some("yanlis-parola")).err().unwrap();
    assert!(e.to_string().contains(backup::WRONG_PASSWORD));
    assert!(store::load_sessions().unwrap().is_empty(), "yanlış parola veri yazmamalı");

    let r = backup::import(&enc, Some("dosya-parolasi")).unwrap();
    assert_eq!((r.added, r.updated, r.secrets, r.keys), (3, 0, 2, 1));
    let sessions = store::load_sessions().unwrap();
    let get = |id: &str| sessions.iter().find(|s| s.id == id).unwrap().clone();
    assert_eq!(store::get_secret(&pw.id).as_deref(), Some("çok gizli ş"));
    assert_eq!(store::get_secret(&keyed.id).as_deref(), Some("anahtar-parolasi"));
    assert!(get(&pw.id).has_secret && !get(&plain.id).has_secret);
    let new_key = get(&keyed.id).key_path.unwrap();
    assert!(new_key.starts_with(dst.to_string_lossy().as_ref()));
    assert_eq!(std::fs::read_to_string(&new_key).unwrap(), "-----BEGIN OPENSSH PRIVATE KEY-----\ntest\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&new_key).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert_eq!(store::load_groups().unwrap(), vec!["Üretim", "Boş"]);
    assert_eq!(r.snippets, 1);
    assert_eq!(store::load_snippets()[0].command, "df -h");

    // Parolasız dosyayı tekrar içe aktarmak güncelleme yapar, kayıtlı parolaları silmez.
    let r = backup::import(&open_file, None).unwrap();
    assert_eq!((r.added, r.updated, r.secrets), (0, 2, 0));
    assert_eq!(store::get_secret(&pw.id).as_deref(), Some("çok gizli ş"));
    assert!(store::load_sessions().unwrap().iter().find(|s| s.id == pw.id).unwrap().has_secret);

    // Başka bir dosya türü reddedilir.
    std::fs::write(src.join("baska.json"), r#"{"format":"x","version":1,"appVersion":"1","encrypted":false}"#).unwrap();
    assert!(backup::import(&src.join("baska.json"), None).is_err());

    let _ = std::fs::remove_dir_all(src);
    let _ = std::fs::remove_dir_all(dst);
}

#[tokio::test]
async fn sftp_only_connection() {
    let Some(mut params) = test_params(None) else {
        return;
    };
    params.shell = false;
    let (out, sink) = Output::new();
    let (on_exit, exited) = exit_signal();
    let term = ssh::connect(params, 80, 24, sink, accept_all(), on_exit).await.unwrap();
    assert!(term.tx.is_none(), "yalnızca SFTP bağlantısında kabuk açılmamalı");
    // Kabuk girdisi sessizce yok sayılır.
    term.send(ssh::SshInput::Data(b"echo x\n".to_vec())).unwrap();

    let s = term.conn.sftp().await.unwrap();
    let home = s.canonicalize(".").await.unwrap();
    sftp::list(s, &home).await.unwrap();

    // Bağlantı kopunca sekmeye haber verilir.
    term.conn.disconnect().await;
    let code = tokio::time::timeout(Duration::from_secs(10), exited)
        .await
        .expect("bağlantı kopması bildirilmedi")
        .unwrap();
    assert_eq!(code, None);
    assert!(!out.text().contains("echo x"));
}

#[tokio::test]
async fn host_key_is_confirmed_once_and_change_is_flagged() {
    let Some(params) = test_params(None) else {
        return;
    };
    let dir = fresh_config("hostkey");
    let connect = |ask: ssh::AskHostKey| {
        let p = test_params(None).unwrap();
        let (_out, sink) = Output::new();
        let (on_exit, _) = exit_signal();
        async move { ssh::connect(p, 80, 24, sink, ask, on_exit).await }
    };
    drop(params);

    // İlk bağlantıda sorulur; reddedilirse bağlanılmaz ve kayıt yapılmaz.
    let (ask, asked) = asker(false);
    let e = connect(ask).await.err().expect("reddedilen anahtarla bağlanmamalı");
    assert!(format!("{e:#}").contains(ssh::HOST_KEY_REJECTED), "{e:#}");
    let q = asked.lock().unwrap()[0].clone();
    assert!(!q.changed && q.fingerprint.starts_with("SHA256:"));
    assert!(!std::fs::read_to_string(dir.join("known_hosts")).unwrap_or_default().contains("2222"));

    // Kabul edilince kaydedilir, sonraki bağlantıda sorulmaz.
    let (ask, asked) = asker(true);
    connect(ask).await.unwrap().conn.disconnect().await;
    assert_eq!(asked.lock().unwrap().len(), 1);
    let (ask, asked) = asker(true);
    connect(ask).await.unwrap().conn.disconnect().await;
    assert!(asked.lock().unwrap().is_empty(), "kayıtlı anahtar için tekrar sorulmamalı");

    // Anahtar değişmişse "değişti" diye sorulur; kabul edilirse eski kayıt silinir.
    let fake = "[127.0.0.1]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl\n";
    std::fs::write(dir.join("known_hosts"), fake).unwrap();
    let (ask, asked) = asker(false);
    assert!(connect(ask).await.is_err());
    assert!(asked.lock().unwrap()[0].changed);
    let (ask, _) = asker(true);
    connect(ask).await.unwrap().conn.disconnect().await;
    let kh = std::fs::read_to_string(dir.join("known_hosts")).unwrap();
    assert_eq!(kh.lines().filter(|l| l.contains("2222")).count(), 1);
    assert!(!kh.contains("AAAAIOMqqnkVzrm0"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn connects_through_jump_host() {
    let Some(jump) = test_params(None) else {
        return;
    };
    let mut target = test_params(None).unwrap();
    // Hedefe atlama sunucusunun gözünden "localhost" ile ulaşılır.
    target.host = "localhost".into();
    target.jump = Some(Box::new(jump));
    let (out, sink) = Output::new();
    let (on_exit, _) = exit_signal();
    let term = ssh::connect(target, 80, 24, sink, accept_all(), on_exit).await.unwrap();
    term.send(ssh::SshInput::Data(b"echo atlama-$((1+1))\n".to_vec())).unwrap();
    out.wait_for("atlama-2").await;
    assert!(out.text().contains("üzerinden localhost:2222"));
    let s = term.conn.sftp().await.unwrap();
    assert!(s.canonicalize(".").await.is_ok());
    term.conn.disconnect().await;
}

/// LIMAN_TEST_AGENT=1 ile, SSH_AUTH_SOCK test anahtarını içeren bir ajanı gösterirken çalışır.
#[tokio::test]
async fn authenticates_with_ssh_agent() {
    if std::env::var("LIMAN_TEST_AGENT").is_err() {
        return;
    }
    let Some(mut params) = test_params(None) else {
        return;
    };
    params.auth = store::AuthKind::Auto;
    params.key_path = None;
    let (_out, sink) = Output::new();
    let (on_exit, _) = exit_signal();
    let term = ssh::connect(params, 80, 24, sink, accept_all(), on_exit)
        .await
        .expect("ssh-agent ile giriş yapılamadı");
    term.conn.disconnect().await;
}

#[test]
fn parses_ssh_config() {
    let cfg = r#"
# genel
User varsayilan

Host bastion
    HostName bastion.example.com
    User ops
    Port 2200

Host web1 web2
  HostName=10.0.0.5
  ProxyJump bastion
  IdentityFile ~/.ssh/id_web

Host db
    HostName db.internal
    ProxyJump ops@bastion.example.com:2200,other
    User "postgres"

Host *.corp *
    Port 2022
    IdentityFile ~/.ssh/id_default

Match host foo
    User yoksay
"#;
    let p = crate::importers::parse_ssh_config(cfg);
    let names: Vec<_> = p.sessions.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["bastion", "web1", "web2", "db"]);
    let get = |n: &str| p.sessions.iter().find(|s| s.name == n).unwrap();
    let b = get("bastion");
    assert_eq!((b.host.as_str(), b.port, b.username.as_str()), ("bastion.example.com", 2200, "ops"));
    assert_eq!(b.key_path.as_deref(), Some("~/.ssh/id_default"));
    let w = get("web2");
    assert_eq!((w.host.as_str(), w.port, w.username.as_str()), ("10.0.0.5", 2022, "varsayilan"));
    assert_eq!(w.key_path.as_deref(), Some("~/.ssh/id_web"));
    assert_eq!(w.jump.as_deref(), Some(b.id.as_str()), "takma ad oturum kimliğine çevrilmeli");
    let d = get("db");
    assert_eq!(d.username, "postgres");
    assert_eq!(d.jump.as_deref(), Some("ops@bastion.example.com:2200,other"));
}

#[test]
fn parses_mobaxterm_sessions() {
    let ini = "\u{feff}[Bookmarks]\r\nSubRep=\r\nImgNum=42\r\nweb= #109#0%10.0.0.1%22%root%%-1%-1%%%22%%0%0%0%%%-1%0%0%0%%1080%%0%0%1#MobaFont%10%0%0%0%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%-1%_Std_Colors_0_%80%24%0%1%-1%<none>%%0#0# #-1\r\n\r\n[Bookmarks_1]\r\nSubRep=Müşteri\\Prod\r\nImgNum=41\r\nfiles= #140#0%files.example.com%2222%deploy%%-1%-1%%#MobaFont%10#0# #-1\r\nuzak masaüstü= #91#4%win.example.com%3389%admin%0%-1#MobaFont#0# #-1\r\n\r\n[Misc]\r\nfoo= #109#0%yoksay%22%x\r\n";
    let p = crate::importers::parse_mobaxterm(ini);
    assert_eq!(p.unsupported, 1);
    assert_eq!(p.sessions.len(), 2);
    let w = &p.sessions[0];
    assert_eq!((w.name.as_str(), w.host.as_str(), w.port, w.username.as_str()), ("web", "10.0.0.1", 22, "root"));
    assert_eq!(w.kind, store::SessionKind::Ssh);
    let f = &p.sessions[1];
    assert_eq!((f.host.as_str(), f.port, f.username.as_str()), ("files.example.com", 2222, "deploy"));
    assert_eq!(f.kind, store::SessionKind::Sftp);
    assert_eq!(f.folder.as_deref(), Some("Müşteri / Prod"));
}

#[test]
fn import_skips_duplicates_and_relinks_jump() {
    let dir = fresh_config("import-dup");
    let existing = store::save_session(session("mevcut", None, store::AuthKind::Auto), None).unwrap();
    let mut dup = session("mevcut", None, store::AuthKind::Auto);
    dup.id = "yeni-kimlik".into();
    let mut child = session("cocuk", None, store::AuthKind::Auto);
    child.id = "cocuk-kimlik".into();
    child.jump = Some("yeni-kimlik".into());
    let r = crate::importers::import(vec![dup, child]).unwrap();
    assert_eq!((r.added, r.skipped), (1, 1));
    let c = store::load_sessions().unwrap().into_iter().find(|s| s.name == "cocuk").unwrap();
    assert_eq!(c.jump.as_deref(), Some(existing.id.as_str()));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn split_pane_shares_connection() {
    let Some(params) = test_params(None) else {
        return;
    };
    let (out1, sink1) = Output::new();
    let (on_exit1, exited1) = exit_signal();
    let first = ssh::connect(params, 80, 24, sink1, accept_all(), on_exit1).await.unwrap();
    let (out2, sink2) = Output::new();
    let (on_exit2, _) = exit_signal();
    let second = first.conn.open_shell(80, 24, sink2, on_exit2).await.unwrap();
    assert!(Arc::ptr_eq(&first.conn, &second.conn));

    first.send(ssh::SshInput::Data(b"echo bir-$((0+1))\n".to_vec())).unwrap();
    second.send(ssh::SshInput::Data(b"echo iki-$((1+1))\n".to_vec())).unwrap();
    out1.wait_for("bir-1").await;
    out2.wait_for("iki-2").await;
    assert!(!out1.text().contains("iki-2") && !out2.text().contains("bir-1"));

    // Bir pano kapanınca diğeri çalışmaya devam eder.
    first.send(ssh::SshInput::Data(b"exit 0\n".to_vec())).unwrap();
    assert_eq!(tokio::time::timeout(Duration::from_secs(10), exited1).await.unwrap().unwrap(), Some(0));
    second.send(ssh::SshInput::Data(b"echo hala-$((2+1))\n".to_vec())).unwrap();
    out2.wait_for("hala-3").await;
    second.conn.disconnect().await;
}

#[test]
fn session_log_strips_escape_sequences() {
    use crate::logger::{SessionLog, Strip};
    let mut out = Vec::new();
    let mut s = Strip::new();
    // Renk, imleç, pencere başlığı (OSC), karakter kümesi ve CR/LF.
    s.feed(b"\x1b]0;baslik\x07\x1b[1;32muser@host\x1b[0m:~$ ls\r\n\x1b(Bdosya \x1b[K", None, &mut out);
    s.feed(b"ikinci\r\n\x1b]7;file://x\x1b\\son", None, &mut out);
    assert_eq!(String::from_utf8(out).unwrap(), "user@host:~$ ls\ndosya ikinci\nson");

    let mut out = Vec::new();
    let mut s = Strip::new();
    s.feed("bir\nİki şğü\n".as_bytes(), Some("[T] "), &mut out);
    assert_eq!(String::from_utf8(out).unwrap(), "[T] bir\n[T] İki şğü\n");

    let dir = std::env::temp_dir().join(format!("liman-log-{}", uuid::Uuid::new_v4()));
    let log = SessionLog::create(dir.clone(), "web/1 prod", false).unwrap();
    log.write(b"\x1b[31mhata\x1b[0m\r\n");
    let name = log.path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("web_1_prod_") && name.ends_with(".log"), "{name}");
    drop(log);
    let text = std::fs::read_to_string(dir.join(&name)).unwrap();
    assert!(text.starts_with("# Liman oturum kaydı: web/1 prod"));
    assert!(text.ends_with("\nhata\n"), "{text:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn local_file_browser() {
    use crate::localfs;
    let dir = std::env::temp_dir().join(format!("liman-lfs-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("Zklasör")).unwrap();
    std::fs::write(dir.join("b.txt"), "12345").unwrap();
    std::fs::write(dir.join("A.txt"), "").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.join("Zklasör"), dir.join("bağ")).unwrap();

    let l = localfs::list(dir.to_str().unwrap()).unwrap();
    let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
    #[cfg(unix)]
    assert_eq!(names, ["bağ", "Zklasör", "A.txt", "b.txt"], "klasörler önce, harf sırasıyla");
    let b = l.entries.iter().find(|e| e.name == "b.txt").unwrap();
    assert_eq!(b.size, 5);
    assert!(l.parent.is_some());
    #[cfg(unix)]
    assert!(l.entries.iter().any(|e| e.name == "bağ" && e.is_link && e.is_dir));

    localfs::mkdir(&l.path, "yeni").unwrap();
    localfs::rename(&format!("{}/b.txt", l.path), "c.txt").unwrap();
    assert!(localfs::rename(&format!("{}/c.txt", l.path), "A.txt").is_err(), "var olanın üzerine yazmamalı");
    localfs::remove(&format!("{}/Zklasör", l.path)).unwrap();
    let names: Vec<_> = localfs::list(&l.path).unwrap().entries.into_iter().map(|e| e.name).collect();
    assert!(names.contains(&"yeni".to_string()) && names.contains(&"c.txt".to_string()));
    assert!(!names.contains(&"Zklasör".to_string()));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn edit_in_local_editor_syncs_back() {
    let Some(params) = test_params(None) else {
        return;
    };
    let (_out, sink) = Output::new();
    let (on_exit, _) = exit_signal();
    let term = ssh::connect(params, 80, 24, sink, accept_all(), on_exit).await.unwrap();
    let s = term.conn.sftp().await.unwrap();
    let remote = ssh::remote_join(&s.canonicalize(".").await.unwrap(), &format!("liman-edit-{}.txt", std::process::id()));
    sftp::write_text(s, &remote, "ilk\n").await.unwrap();

    let local = sftp::fetch_for_edit(s, &remote).await.unwrap();
    assert_eq!(std::fs::read_to_string(&local).unwrap(), "ilk\n");
    let reports = Arc::new(Mutex::new(Vec::new()));
    let r = reports.clone();
    let task = tokio::spawn(sftp::sync_edits(Arc::downgrade(&term.conn), local.clone(), remote.clone(), move |res| {
        r.lock().unwrap().push(res.is_ok())
    }));

    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(reports.lock().unwrap().is_empty(), "değişiklik yokken yüklememeli");
    std::fs::write(&local, "düzenlendi ş\n").unwrap();
    for _ in 0..50 {
        if !reports.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(*reports.lock().unwrap(), vec![true]);
    assert_eq!(sftp::read_text(s, &remote).await.unwrap(), "düzenlendi ş\n");

    // Bağlantı kapanınca izleme durur ve geçici klasör silinir.
    s.remove_file(remote).await.unwrap();
    term.conn.disconnect().await;
    drop(term);
    tokio::time::timeout(Duration::from_secs(5), task).await.expect("izleme durmadı").unwrap();
    assert!(!local.parent().unwrap().exists());
}

#[tokio::test]
async fn folder_compare_and_sync() {
    use crate::sync::{self, Action, Direction, Status};
    let Some(params) = test_params(None) else {
        return;
    };
    let (_out, sink) = Output::new();
    let (on_exit, _) = exit_signal();
    let term = ssh::connect(params, 80, 24, sink, accept_all(), on_exit).await.unwrap();
    let s = term.conn.sftp().await.unwrap();
    let remote = ssh::remote_join(&s.canonicalize(".").await.unwrap(), &format!("liman-sync-{}", std::process::id()));
    let local = std::env::temp_dir().join(format!("liman-sync-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(local.join("alt/derin")).unwrap();
    std::fs::write(local.join("yalniz-yerel.txt"), "y").unwrap();
    std::fs::write(local.join("alt/derin/ortak.txt"), "yerel").unwrap();
    s.create_dir(remote.clone()).await.unwrap();
    s.create_dir(format!("{remote}/alt")).await.unwrap();
    sftp::write_text(s, &format!("{remote}/yalniz-uzak.txt"), "u").await.unwrap();

    let dummy = |_: sftp::Progress| {};
    let c = sync::compare(s, &local, &remote).await.unwrap();
    let st = |p: &str| c.diffs.iter().find(|d| d.path == p).map(|d| d.status);
    assert_eq!(st("yalniz-yerel.txt"), Some(Status::OnlyLocal));
    assert_eq!(st("alt/derin/ortak.txt"), Some(Status::OnlyLocal));
    assert_eq!(st("yalniz-uzak.txt"), Some(Status::OnlyRemote));
    assert!(!c.truncated);

    for a in [
        Action { path: "alt/derin/ortak.txt".into(), direction: Direction::Upload },
        Action { path: "yalniz-yerel.txt".into(), direction: Direction::Upload },
        Action { path: "yalniz-uzak.txt".into(), direction: Direction::Download },
    ] {
        let mut rep = sftp::Reporter::new("x".into(), "x".into(), dummy);
        sync::apply_one(s, &local, &remote, &a, &mut rep).await.unwrap();
    }
    // Zamanlar korunduğu için eşitlemeden sonra fark kalmamalı.
    let c = sync::compare(s, &local, &remote).await.unwrap();
    assert!(c.diffs.is_empty(), "{:?}", c.diffs);
    assert_eq!(c.same, 3);
    assert_eq!(sftp::read_text(s, &format!("{remote}/alt/derin/ortak.txt")).await.unwrap(), "yerel");
    assert_eq!(std::fs::read_to_string(local.join("yalniz-uzak.txt")).unwrap(), "u");

    // Yerel dosya değişince "yerel daha yeni" görünür.
    std::fs::write(local.join("yalniz-yerel.txt"), "degisti").unwrap();
    let f = std::fs::File::options().write(true).open(local.join("yalniz-yerel.txt")).unwrap();
    f.set_modified(std::time::SystemTime::now() + Duration::from_secs(60)).unwrap();
    let c = sync::compare(s, &local, &remote).await.unwrap();
    assert_eq!(c.diffs.len(), 1);
    assert_eq!(c.diffs[0].status, Status::LocalNewer);

    // Kökten çıkmaya çalışan yol reddedilir.
    let mut rep = sftp::Reporter::new("x".into(), "x".into(), dummy);
    let bad = Action { path: "../kacak.txt".into(), direction: Direction::Download };
    assert!(sync::apply_one(s, &local, &remote, &bad, &mut rep).await.is_err());

    sftp::remove(s, remote, true).await.unwrap();
    term.conn.disconnect().await;
    let _ = std::fs::remove_dir_all(local);
}

async fn echo_server() -> u16 {
    let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = echo.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = sock.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    port
}

async fn roundtrip(sock: &mut tokio::net::TcpStream, msg: &[u8]) {
    sock.write_all(msg).await.unwrap();
    let mut buf = vec![0u8; msg.len()];
    tokio::time::timeout(Duration::from_secs(5), sock.read_exact(&mut buf)).await.unwrap().unwrap();
    assert_eq!(buf, msg);
}

#[tokio::test]
async fn socks_and_remote_forwarding() {
    let Some(params) = test_params(None) else {
        return;
    };
    let (_out, sink) = Output::new();
    let (on_exit, _) = exit_signal();
    let term = ssh::connect(params, 80, 24, sink, accept_all(), on_exit).await.unwrap();
    let echo = echo_server().await;

    // SOCKS5: alan adıyla (localhost) hedef isteği.
    let socks = term.conn.start_tunnel(ssh::TunnelKind::Socks, String::new(), 0, String::new(), 0).await.unwrap();
    let mut c = tokio::net::TcpStream::connect(("127.0.0.1", socks.listen_port)).await.unwrap();
    c.write_all(&[5, 1, 0]).await.unwrap();
    let mut r = [0u8; 2];
    c.read_exact(&mut r).await.unwrap();
    assert_eq!(r, [5, 0]);
    let mut req = vec![5, 1, 0, 3, 9];
    req.extend_from_slice(b"localhost");
    req.extend_from_slice(&echo.to_be_bytes());
    c.write_all(&req).await.unwrap();
    let mut rep = [0u8; 10];
    c.read_exact(&mut rep).await.unwrap();
    assert_eq!(rep[1], 0, "SOCKS bağlantısı başarılı olmalı");
    roundtrip(&mut c, b"socks-ok").await;

    // Ulaşılamayan hedefte hata kodu döner.
    let mut c2 = tokio::net::TcpStream::connect(("127.0.0.1", socks.listen_port)).await.unwrap();
    c2.write_all(&[5, 1, 0]).await.unwrap();
    c2.read_exact(&mut r).await.unwrap();
    c2.write_all(&[5, 1, 0, 1, 127, 0, 0, 1, 0, 1]).await.unwrap();
    let mut rep = [0u8; 10];
    c2.read_exact(&mut rep).await.unwrap();
    assert_ne!(rep[1], 0);

    // Uzak yönlendirme: sunucudaki port → bu makinedeki yankı sunucusu.
    let remote = term
        .conn
        .start_tunnel(ssh::TunnelKind::Remote, "localhost".into(), 0, "127.0.0.1".into(), echo)
        .await
        .unwrap();
    assert!(remote.listen_port > 0);
    let mut c3 = tokio::net::TcpStream::connect(("127.0.0.1", remote.listen_port)).await.unwrap();
    roundtrip(&mut c3, b"uzak-ok").await;
    assert_eq!(term.conn.tunnels().len(), 2);

    term.conn.stop_tunnel(&remote.id).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(tokio::net::TcpStream::connect(("127.0.0.1", remote.listen_port)).await.is_err(), "sunucu artık dinlememeli");
    term.conn.stop_tunnel(&socks.id).await;
    assert!(term.conn.tunnels().is_empty());
    term.conn.disconnect().await;
}

#[test]
fn telnet_negotiation() {
    use crate::raw::{escape_input, naws, TelnetParser};
    let mut p = TelnetParser::default();
    let (mut out, mut reply) = (Vec::new(), Vec::new());
    // DO NAWS, DO TTYPE, WILL ECHO, DO (bilinmeyen 39), metin, kaçırılmış 255, SB TTYPE SEND.
    let input = [
        255, 253, 31, 255, 253, 24, 255, 251, 1, 255, 253, 39, b'h', b'i', 255, 255, 255, 250, 24, 1, 255, 240, b'!',
    ];
    // Paketin ortasından bölünmüş gelse de aynı sonuç.
    p.feed(&input[..10], (100, 40), &mut out, &mut reply);
    p.feed(&input[10..], (100, 40), &mut out, &mut reply);
    assert_eq!(out, [b'h', b'i', 255, b'!']);
    assert!(p.naws);
    let mut expected = vec![255, 251, 31];
    expected.extend(naws(100, 40));
    expected.extend([255, 251, 24, 255, 253, 1, 255, 252, 39]);
    expected.extend([255, 250, 24, 0]);
    expected.extend(b"xterm-256color");
    expected.extend([255, 240]);
    assert_eq!(reply, expected);
    assert_eq!(naws(80, 255), vec![255, 250, 31, 0, 80, 0, 255, 255, 255, 240]);
    assert_eq!(escape_input(&[1, 255, 2]), vec![1, 255, 255, 2]);
}

#[tokio::test]
async fn telnet_session_roundtrip() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // Basit bir telnet sunucusu: NAWS ister, bir karşılama yazar, gelen satırı yankılar.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        s.write_all(&[255, 253, 31]).await.unwrap();
        s.write_all(b"merhaba\r\n").await.unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 256];
        while !got.ends_with(b"\r") {
            let n = s.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        s.write_all(b"tamam\r\n").await.unwrap();
        got
    });
    let (out, sink) = Output::new();
    let (on_exit, exited) = exit_signal();
    let t = crate::raw::telnet("127.0.0.1", port, 90, 30, sink, on_exit).await.unwrap();
    out.wait_for("merhaba").await;
    t.send(crate::raw::RawInput::Data(b"ls\r".to_vec())).unwrap();
    out.wait_for("tamam").await;
    let got = server.await.unwrap();
    // İstemci önce WILL NAWS ve boyutu, sonra komutu göndermiş olmalı.
    let mut expected = vec![255, 251, 31];
    expected.extend(crate::raw::naws(90, 30));
    expected.extend(b"ls\r");
    assert_eq!(got, expected);
    assert!(!out.text().contains('\u{fffd}'), "telnet komutları ekrana düşmemeli");
    // Sunucu kapanınca bildirilir.
    tokio::time::timeout(Duration::from_secs(5), exited).await.unwrap().unwrap();
}

/// Seri port: donanım yerine sanal bir terminal çifti (pty) kullanılır. macOS'ta pty'ler
/// seri port hız ayarını (IOSSIOSPEED) desteklemediği için yalnızca Linux'ta çalışır.
#[cfg(target_os = "linux")]
#[test]
fn serial_port_over_pty() {
    use portable_pty::{native_pty_system, PtySize};
    use std::io::{Read, Write};
    let pair = native_pty_system()
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let path = pair.master.tty_name().expect("pty yolu yok");
    let mut peer_w = pair.master.take_writer().unwrap();
    let mut peer_r = pair.master.try_clone_reader().unwrap();

    let (out, sink) = Output::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let t = crate::raw::serial(
        path.to_str().unwrap(),
        115200,
        sink,
        Box::new(move |_| {
            let _ = tx.send(());
        }),
    )
    .unwrap();

    // Aygıttan gelen veri terminale ulaşır.
    peer_w.write_all(b"aygit-hazir\n").unwrap();
    peer_w.flush().unwrap();
    for _ in 0..50 {
        if out.text().contains("aygit-hazir") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(out.text().contains("aygit-hazir"), "{:?}", out.text());

    // Yazılan aygıta gider.
    t.send(crate::raw::RawInput::Data(b"AT\r".to_vec())).unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 64];
    while !got.windows(2).any(|w| w == b"AT") {
        let n = peer_r.read(&mut buf).unwrap();
        got.extend_from_slice(&buf[..n]);
    }
    t.send(crate::raw::RawInput::Close).unwrap();
    rx.recv_timeout(Duration::from_secs(5)).expect("kapanış bildirilmedi");
    drop(pair.slave);
}

#[test]
fn x11_display_and_cookie_rewrite() {
    use crate::x11::{cookie_from_xauth, parse_display, rewrite_setup, Display, AUTH_PROTO};
    #[cfg(unix)]
    assert_eq!(parse_display(":0"), Some(Display::Unix("/tmp/.X11-unix/X0".into())));
    #[cfg(unix)]
    assert_eq!(parse_display("unix:1.0"), Some(Display::Unix("/tmp/.X11-unix/X1".into())));
    assert_eq!(parse_display("localhost:10.0"), Some(Display::Tcp("localhost".into(), 6010)));
    assert_eq!(
        parse_display("/private/tmp/com.apple.launchd.x/org.xquartz:0"),
        Some(Display::Unix("/private/tmp/com.apple.launchd.x/org.xquartz:0".into()))
    );
    assert_eq!(parse_display("bozuk"), None);

    let xauth = "makine/unix:0  MIT-MAGIC-COOKIE-1  00ff10ab\nbaska:1  XDM-AUTHORIZATION-1  1234\n";
    assert_eq!(cookie_from_xauth(xauth), Some(vec![0x00, 0xff, 0x10, 0xab]));

    let fake = [7u8; 16];
    let setup = |le: bool, name: &[u8], data: &[u8]| {
        let p = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut v = vec![if le { b'l' } else { b'B' }, 0];
        v.extend(p(11));
        v.extend(p(0));
        v.extend(p(name.len() as u16));
        v.extend(p(data.len() as u16));
        v.extend([0, 0]);
        v.extend(name);
        v.extend(std::iter::repeat_n(0, (4 - name.len() % 4) % 4));
        v.extend(data);
        v.extend(std::iter::repeat_n(0, (4 - data.len() % 4) % 4));
        v
    };
    for le in [true, false] {
        let mut pkt = setup(le, AUTH_PROTO.as_bytes(), &fake);
        pkt.extend(b"sonrasi");
        // Eksik paket: daha fazla veri beklenir.
        assert!(rewrite_setup(&pkt[..20], &fake, None).unwrap().is_none());
        let real = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let mut want = setup(le, AUTH_PROTO.as_bytes(), &real);
        want.extend(b"sonrasi");
        assert_eq!(rewrite_setup(&pkt, &fake, Some(&real)).unwrap().unwrap(), want);
        let mut want = setup(le, b"", b"");
        want.extend(b"sonrasi");
        assert_eq!(rewrite_setup(&pkt, &fake, None).unwrap().unwrap(), want);
        // Yanlış çerez reddedilir.
        let bad = setup(le, AUTH_PROTO.as_bytes(), &[9u8; 16]);
        assert!(rewrite_setup(&bad, &fake, None).is_err());
    }
}

#[tokio::test]
async fn raw_streams_direct_and_over_ssh() {
    use crate::stream::Streams;
    let echo = echo_server().await;
    let streams = Streams::default();

    // Doğrudan TCP.
    let (out, sink) = Output::new();
    let (on_exit, exited) = exit_signal();
    streams.open("a".into(), "127.0.0.1", echo, None, sink, on_exit).await.unwrap();
    streams.write("a", b"dogrudan".to_vec()).unwrap();
    out.wait_for("dogrudan").await;
    streams.close("a");
    tokio::time::timeout(Duration::from_secs(5), exited).await.unwrap().unwrap();
    assert!(streams.write("a", b"x".to_vec()).is_err());

    // SSH üzerinden (VNC tüneli gibi); bağlantı kabuksuz ve SFTP'siz.
    let Some(mut params) = test_params(None) else {
        return;
    };
    params.shell = false;
    params.tunnel_only = true;
    let (_o, s) = Output::new();
    let (e, _) = exit_signal();
    let term = ssh::connect(params, 80, 24, s, accept_all(), e).await.unwrap();
    let (out, sink) = Output::new();
    let (on_exit, _) = exit_signal();
    streams.open("b".into(), "localhost", echo, Some(&term.conn), sink, on_exit).await.unwrap();
    streams.write("b", b"tunelden".to_vec()).unwrap();
    out.wait_for("tunelden").await;
    streams.close("b");
    term.conn.disconnect().await;
}

#[test]
fn parses_linux_server_stats() {
    use crate::stats::{compute, parse};
    let sample = |cpu: &str, rx: u64, tx: u64| {
        format!(
            "@os\nLinux\n@cpu\n{cpu}\n@mem\nMemTotal:       16303544 kB\nMemFree:         1203544 kB\nMemAvailable:    8151772 kB\nBuffers:          300000 kB\nCached:          5000000 kB\nSwapTotal:       2097148 kB\nSwapFree:        1048574 kB\n@load\n0.52 0.41 0.30 2/812 31337\n@uptime\n356521.42\n@ncpu\n8\n@net\n    lo: 999999 100 0 0 0 0 0 0 999999 100 0 0 0 0 0 0\n  eth0: {rx} 2000 0 0 0 0 0 0 {tx} 1500 0 0 0 0 0 0\n docker0: 1000 10 0 0 0 0 0 0 2000 10 0 0 0 0 0 0\n@df\nFilesystem     1024-blocks      Used Available Capacity Mounted on\n/dev/sda1        102400000  61440000  40960000      60% /\ntmpfs              8151772         0   8151772       0% /dev/shm\n/dev/loop3           56832     56832         0     100% /snap/core/1\n/dev/sdb1        512000000 128000000 384000000      25% /srv/veri alanı\nnas:/export      1000000   500000    500000      50% /mnt/nas\n"
        )
    };
    let mut a = parse(&sample("cpu  1000 0 1000 7000 1000 0 0 0 0 0", 1_000_000, 500_000));
    a.at = Some(std::time::Instant::now());
    assert_eq!(a.os, "Linux");
    assert_eq!(a.mem_total, 16303544 * 1024);
    assert_eq!(a.mem_used, (16303544 - 8151772) * 1024);
    assert_eq!(a.swap_used, (2097148 - 1048574) * 1024);
    assert_eq!(a.load, Some([0.52, 0.41, 0.30]));
    assert_eq!(a.uptime, Some(356521));
    assert_eq!(a.ncpu, Some(8));
    let mounts: Vec<_> = a.disks.iter().map(|d| d.mount.as_str()).collect();
    assert_eq!(mounts, ["/", "/srv/veri alanı", "/mnt/nas"], "tmpfs ve snap döngüleri elenir, boşluklu yol korunur");

    // İlk ölçümde CPU ve ağ hızı bilinmez.
    let first = compute(None, &a);
    assert_eq!(first.cpu, None);
    assert_eq!(first.net_rx, None);
    assert_eq!(first.disk.as_ref().unwrap().mount, "/");
    assert_eq!(first.disk.as_ref().unwrap().used, 61440000 * 1024);

    // 2 sn sonra: toplam +1000 jiffy, boşta (idle+iowait) +250 → %75; eth0+docker0 hızları.
    let mut b = parse(&sample("cpu  1500 0 1250 7200 1050 0 0 0 0 0", 1_000_000 + 4_000, 500_000 + 2_000));
    b.at = Some(a.at.unwrap() + Duration::from_secs(2));
    let s = compute(Some(&a), &b);
    assert!((s.cpu.unwrap() - 75.0).abs() < 0.01, "{:?}", s.cpu);
    assert!((s.net_rx.unwrap() - 2000.0).abs() < 0.01);
    assert!((s.net_tx.unwrap() - 1000.0).abs() < 0.01);

    // Sayaç geri giderse (arayüz yeniden başladı) hız gösterilmez.
    let mut c = parse(&sample("cpu  1600 0 1300 7300 1060 0 0 0 0 0", 10, 10));
    c.at = Some(b.at.unwrap() + Duration::from_secs(2));
    assert_eq!(compute(Some(&b), &c).net_rx, None);
}

#[tokio::test]
async fn live_server_stats() {
    use crate::stats::{compute, parse, SCRIPT};
    let Some(params) = test_params(None) else {
        return;
    };
    let (out, sink) = Output::new();
    let (on_exit, _) = exit_signal();
    let term = ssh::connect(params, 80, 24, sink, accept_all(), on_exit).await.unwrap();
    let run = || term.conn.exec_capture("sh -s", SCRIPT.as_bytes(), Duration::from_secs(8));
    let mut a = parse(&run().await.unwrap());
    a.at = Some(std::time::Instant::now());
    tokio::time::sleep(Duration::from_millis(500)).await;
    let raw = run().await.unwrap();
    let mut b = parse(&raw);
    b.at = Some(std::time::Instant::now());
    let s = compute(Some(&a), &b);
    eprintln!("{s:#?}");
    assert!(!s.os.is_empty(), "{raw}");
    assert!(s.cpu.is_some_and(|c| (0.0..=100.0).contains(&c)), "{raw}");
    assert!(s.mem_total > 0 && s.mem_used > 0 && s.mem_used <= s.mem_total, "{raw}");
    assert!(s.load.is_some() && s.uptime.is_some_and(|u| u > 0) && s.ncpu.is_some_and(|n| n > 0), "{raw}");
    let d = s.disk.expect("ana disk yok");
    assert!(d.total > 0 && d.used <= d.total);
    // Ölçüm kabuğa hiçbir şey yazmamalı.
    assert!(!out.text().contains("@os"));
    term.conn.disconnect().await;
}
