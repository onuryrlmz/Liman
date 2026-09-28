//! Oturum kaydı: terminal çıktısını renk/imleç kodlarından arındırıp düz metin dosyasına yazar.

use std::{
    fs::{self, File},
    io::{LineWriter, Write},
    path::PathBuf,
    sync::Mutex,
};

use chrono::Local;

use crate::store;

/// ANSI kaçış dizilerini ayıklayan küçük durum makinesi.
#[derive(Default)]
pub struct Strip {
    state: State,
    /// Satır başında mıyız? (zaman damgası için)
    line_start: bool,
}

#[derive(Default, PartialEq)]
enum State {
    #[default]
    Text,
    Esc,
    /// ESC [ … son bayt 0x40–0x7E
    Csi,
    /// ESC ] … BEL ya da ESC \
    Osc,
    OscEsc,
    /// ESC ( B gibi iki baytlık diziler
    Skip,
}

impl Strip {
    pub fn new() -> Self {
        Self {
            state: State::Text,
            line_start: true,
        }
    }

    /// `stamp`: her satırın başına eklenecek metin (zaman damgası).
    pub fn feed(&mut self, data: &[u8], stamp: Option<&str>, out: &mut Vec<u8>) {
        for &b in data {
            self.state = match self.state {
                State::Text => match b {
                    0x1b => State::Esc,
                    b'\n' => {
                        out.push(b'\n');
                        self.line_start = true;
                        State::Text
                    }
                    // Satır başı, zil ve diğer denetim karakterleri kayda girmez.
                    b'\r' | 0x07 | 0x00..=0x08 | 0x0b..=0x1a | 0x1c..=0x1f | 0x7f => State::Text,
                    _ => {
                        if self.line_start {
                            if let Some(s) = stamp {
                                out.extend_from_slice(s.as_bytes());
                            }
                            self.line_start = false;
                        }
                        out.push(b);
                        State::Text
                    }
                },
                State::Esc => match b {
                    b'[' => State::Csi,
                    b']' | b'P' | b'_' | b'^' => State::Osc,
                    b'(' | b')' | b'*' | b'+' | b'#' | b'%' => State::Skip,
                    _ => State::Text,
                },
                State::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        State::Text
                    } else {
                        State::Csi
                    }
                }
                State::Osc => match b {
                    0x07 => State::Text,
                    0x1b => State::OscEsc,
                    _ => State::Osc,
                },
                State::OscEsc => {
                    if b == b'\\' {
                        State::Text
                    } else {
                        State::Osc
                    }
                }
                State::Skip => State::Text,
            };
        }
    }
}

pub struct SessionLog {
    inner: Mutex<(LineWriter<File>, Strip)>,
    timestamps: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub path: PathBuf,
}

fn safe_name(s: &str) -> String {
    let n: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || "-_.@".contains(c) { c } else { '_' })
        .collect();
    n.trim_matches('_').chars().take(60).collect()
}

pub fn default_dir() -> PathBuf {
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Liman Kayıtları")
}

impl SessionLog {
    /// Ayarlarda oturum kaydı açıksa yeni bir kayıt dosyası açar.
    pub fn open(name: &str) -> Option<Self> {
        let settings = store::load_settings();
        if !settings["sessionLog"].as_bool().unwrap_or(false) {
            return None;
        }
        let dir = settings["logDir"]
            .as_str()
            .filter(|d| !d.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_dir);
        let timestamps = settings["logTimestamps"].as_bool().unwrap_or(true);
        Self::create(dir, name, timestamps).ok()
    }

    pub fn create(dir: PathBuf, name: &str, timestamps: bool) -> std::io::Result<Self> {
        fs::create_dir_all(&dir)?;
        let now = Local::now();
        let base = format!("{}_{}", safe_name(name), now.format("%Y-%m-%d_%H-%M-%S"));
        let mut path = dir.join(format!("{base}.log"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{base}_{n}.log"));
            n += 1;
        }
        let mut file = LineWriter::new(File::create(&path)?);
        writeln!(file, "# Liman oturum kaydı: {name} — {}", now.format("%Y-%m-%d %H:%M:%S"))?;
        Ok(Self {
            inner: Mutex::new((file, Strip::new())),
            timestamps,
            path,
        })
    }

    pub fn write(&self, data: &[u8]) {
        let stamp = self
            .timestamps
            .then(|| Local::now().format("[%H:%M:%S] ").to_string());
        let mut guard = self.inner.lock().unwrap();
        let (file, strip) = &mut *guard;
        let mut out = Vec::with_capacity(data.len());
        strip.feed(data, stamp.as_deref(), &mut out);
        let _ = file.write_all(&out);
    }
}
