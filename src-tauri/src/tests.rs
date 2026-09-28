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
        Box::new(move |_| {
            let _ = tx.send(());
        }),
    )
    .unwrap();
    term.write(b"echo liman-$((40+2))\r").unwrap();
    term.resize(120, 30).unwrap();
    term.write(b"exit\r").unwrap();
    rx.recv_timeout(Duration::from_secs(10)).expect("kabuk kapanmadı");
    assert!(out.text().contains("liman-42"), "{}", out.text());
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
    let term = ssh::connect(params, 100, 30, sink, on_exit).await.unwrap();
    term.tx
        .send(ssh::SshInput::Data(b"echo liman-$((40+2))\n".to_vec()))
        .unwrap();
    out.wait_for("liman-42").await;
    term.tx.send(ssh::SshInput::Resize(132, 40)).unwrap();
    term.tx
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
    term.tx
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
    let err = ssh::connect(params, 80, 24, sink, on_exit)
        .await
        .err()
        .expect("yanlış anahtarla bağlanmamalıydı");
    assert!(err.to_string().contains(ssh::AUTH_FAILED), "{err}");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("pub"));
}
