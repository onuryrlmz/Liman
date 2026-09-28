//! Uçtan uca testler. SSH testleri gerçek bir sshd ister:
//!
//! LIMAN_TEST_SSH="127.0.0.1:2222:kullanici:/yol/ozel_anahtar" cargo test
//!
//! Değişken tanımlı değilse SSH testleri atlanır.

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
        .start_tunnel(0, "127.0.0.1".into(), echo_port)
        .await
        .unwrap();
    assert_eq!(term.conn.tunnels().len(), 1);
    let mut c = tokio::net::TcpStream::connect(("127.0.0.1", info.local_port))
        .await
        .unwrap();
    c.write_all(b"tunel-ok").await.unwrap();
    let mut buf = [0u8; 8];
    tokio::time::timeout(Duration::from_secs(5), c.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"tunel-ok");
    term.conn.stop_tunnel(&info.id);
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
