//! Telnet ve seri port terminalleri.

use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

use crate::ssh::{OnExit, Sink};

pub enum RawInput {
    Data(Vec<u8>),
    Resize(u16, u16),
    Close,
}

pub struct RawTerm {
    tx: mpsc::UnboundedSender<RawInput>,
}

impl RawTerm {
    pub fn send(&self, input: RawInput) -> Result<()> {
        self.tx.send(input).map_err(|_| anyhow!("Bağlantı kapalı"))
    }
}

// ---------- Telnet ----------
// RFC 854 temel protokol; pencere boyutu (NAWS, RFC 1073) ve terminal türü (RFC 1091) bildirilir.

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;

/// Sunucudan gelen akıştan telnet komutlarını ayıklar ve gereken yanıtları üretir.
#[derive(Default)]
pub struct TelnetParser {
    state: TState,
    sb: Vec<u8>,
    /// Sunucu NAWS istediyse boyut değişikliklerini bildiririz.
    pub naws: bool,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum TState {
    #[default]
    Data,
    Iac,
    Cmd(u8),
    Sb,
    SbIac,
}

pub fn naws(cols: u16, rows: u16) -> Vec<u8> {
    let mut v = vec![IAC, SB, OPT_NAWS];
    for b in cols.to_be_bytes().into_iter().chain(rows.to_be_bytes()) {
        v.push(b);
        // Veri içindeki 255 iki kez yazılır.
        if b == IAC {
            v.push(IAC);
        }
    }
    v.extend([IAC, SE]);
    v
}

impl TelnetParser {
    /// `data`'yı işler; ekrana gidecek baytları `out`'a, sunucuya yanıtları `reply`'ye yazar.
    pub fn feed(&mut self, data: &[u8], size: (u16, u16), out: &mut Vec<u8>, reply: &mut Vec<u8>) {
        for &b in data {
            self.state = match self.state {
                TState::Data if b == IAC => TState::Iac,
                TState::Data => {
                    out.push(b);
                    TState::Data
                }
                TState::Iac => match b {
                    IAC => {
                        out.push(IAC);
                        TState::Data
                    }
                    DO | DONT | WILL | WONT => TState::Cmd(b),
                    SB => {
                        self.sb.clear();
                        TState::Sb
                    }
                    _ => TState::Data,
                },
                TState::Cmd(cmd) => {
                    match (cmd, b) {
                        (DO, OPT_NAWS) => {
                            self.naws = true;
                            reply.extend([IAC, WILL, OPT_NAWS]);
                            reply.extend(naws(size.0, size.1));
                        }
                        (DO, OPT_TTYPE) => reply.extend([IAC, WILL, OPT_TTYPE]),
                        (DO, _) => reply.extend([IAC, WONT, b]),
                        (WILL, OPT_ECHO | OPT_SGA) => reply.extend([IAC, DO, b]),
                        (WILL, _) => reply.extend([IAC, DONT, b]),
                        _ => {}
                    }
                    TState::Data
                }
                TState::Sb if b == IAC => TState::SbIac,
                TState::Sb => {
                    self.sb.push(b);
                    TState::Sb
                }
                TState::SbIac if b == SE => {
                    // TTYPE SEND → TTYPE IS xterm-256color
                    if self.sb.first() == Some(&OPT_TTYPE) && self.sb.get(1) == Some(&1) {
                        reply.extend([IAC, SB, OPT_TTYPE, 0]);
                        reply.extend(b"xterm-256color");
                        reply.extend([IAC, SE]);
                    }
                    TState::Data
                }
                TState::SbIac => {
                    self.sb.push(b);
                    TState::Sb
                }
            };
        }
    }
}

/// Kullanıcı girdisinde 255 baytını kaçırır.
pub fn escape_input(data: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(data.len());
    for &b in data {
        v.push(b);
        if b == IAC {
            v.push(IAC);
        }
    }
    v
}

pub async fn telnet(host: &str, port: u16, cols: u16, rows: u16, out: Sink, on_exit: OnExit) -> Result<RawTerm> {
    let sock = tokio::time::timeout(Duration::from_secs(15), tokio::net::TcpStream::connect((host, port)))
        .await
        .map_err(|_| anyhow!("{host}:{port} bağlantısı zaman aşımına uğradı"))?
        .with_context(|| format!("{host}:{port} adresine bağlanılamadı"))?;
    let (mut rd, mut wr) = sock.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<RawInput>();
    let size = Arc::new(Mutex::new((cols, rows)));
    let naws_on = Arc::new(Mutex::new(false));
    // Sunucuya giden yanıtlar ile kullanıcı girdisi aynı yazıcıdan geçer; `None` bağlantıyı kapatır.
    let (wtx, mut wrx) = mpsc::unbounded_channel::<Option<Vec<u8>>>();

    tokio::spawn(async move {
        while let Some(Some(buf)) = wrx.recv().await {
            if wr.write_all(&buf).await.is_err() {
                break;
            }
        }
        let _ = wr.shutdown().await;
    });

    {
        let wtx = wtx.clone();
        let size = size.clone();
        let naws_on = naws_on.clone();
        tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    RawInput::Data(d) => {
                        let _ = wtx.send(Some(escape_input(&d)));
                    }
                    RawInput::Resize(c, r) => {
                        *size.lock().unwrap() = (c, r);
                        if *naws_on.lock().unwrap() {
                            let _ = wtx.send(Some(naws(c, r)));
                        }
                    }
                    RawInput::Close => {
                        let _ = wtx.send(None);
                        break;
                    }
                }
            }
        });
    }

    tokio::spawn(async move {
        let mut parser = TelnetParser::default();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let (mut shown, mut reply) = (Vec::new(), Vec::new());
            let sz = *size.lock().unwrap();
            parser.feed(&buf[..n], sz, &mut shown, &mut reply);
            *naws_on.lock().unwrap() = parser.naws;
            if !reply.is_empty() {
                let _ = wtx.send(Some(reply));
            }
            if !shown.is_empty() {
                out(shown);
            }
        }
        on_exit(None);
    });

    Ok(RawTerm { tx })
}

// ---------- Seri port ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialPortDesc {
    pub path: String,
    /// USB aygıtlarında üretici/ürün bilgisi.
    pub description: String,
}

pub fn serial_ports() -> Vec<SerialPortDesc> {
    let mut v: Vec<SerialPortDesc> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| SerialPortDesc {
            description: match &p.port_type {
                serialport::SerialPortType::UsbPort(u) => [u.manufacturer.clone(), u.product.clone()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" "),
                serialport::SerialPortType::BluetoothPort => "Bluetooth".into(),
                _ => String::new(),
            },
            path: p.port_name,
        })
        .collect();
    // macOS'ta her aygıt hem tty. hem cu. olarak görünür; bağlanmak için cu. doğrusudur.
    #[cfg(target_os = "macos")]
    v.retain(|p| !p.path.starts_with("/dev/tty."));
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v
}

pub fn serial(path: &str, baud: u32, out: Sink, on_exit: OnExit) -> Result<RawTerm> {
    let port = serialport::new(path, baud)
        .timeout(Duration::from_millis(100))
        .open()
        .with_context(|| format!("{path} açılamadı (başka bir uygulama kullanıyor ya da izin yok olabilir)"))?;
    let mut reader = port.try_clone().context("Seri port kopyalanamadı")?;
    let mut writer = port;
    let (tx, mut rx) = mpsc::unbounded_channel::<RawInput>();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let stop_r = stop.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while !stop_r.load(std::sync::atomic::Ordering::Relaxed) {
            match reader.read(&mut buf) {
                Ok(0) => continue,
                Ok(n) => out(buf[..n].to_vec()),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                // Aygıt çıkarıldı.
                Err(_) => break,
            }
        }
        on_exit(None);
    });

    std::thread::spawn(move || {
        while let Some(cmd) = rx.blocking_recv() {
            match cmd {
                RawInput::Data(d) => {
                    if writer.write_all(&d).is_err() {
                        break;
                    }
                }
                RawInput::Resize(..) => {}
                RawInput::Close => break,
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    });

    Ok(RawTerm { tx })
}
