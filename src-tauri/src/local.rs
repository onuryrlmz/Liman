//! Yerel kabuk: macOS/Linux'ta $SHELL, Windows'ta PowerShell.

use std::{
    io::{Read, Write},
    sync::mpsc,
    time::Duration,
};

use anyhow::Result;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use crate::ssh::{OnExit, Sink};

pub struct LocalTerm {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

impl LocalTerm {
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(size(cols, rows))
    }

    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

pub fn spawn(
    shell: Option<String>,
    cols: u16,
    rows: u16,
    out: Sink,
    on_exit: OnExit,
) -> Result<LocalTerm> {
    let pair = native_pty_system().openpty(size(cols, rows))?;
    let mut cmd = match shell.filter(|s| !s.is_empty()) {
        Some(s) => {
            let mut parts = s.split_whitespace();
            let mut c = CommandBuilder::new(parts.next().unwrap_or_default());
            c.args(parts);
            c
        }
        None => default_shell(),
    };
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "Liman");
    if let Some(home) = dirs::home_dir() {
        cmd.cwd(home);
    }
    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);
    let killer = child.clone_killer();

    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let (reader_done, reader_finished) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => out(buf[..n].to_vec()),
            }
        }
        let _ = reader_done.send(());
    });
    // Kapanışı süreçten algıla: Windows'ta (ConPTY) kabuk çıksa da okuma ucu açık kalır.
    std::thread::spawn(move || {
        let code = child.wait().ok().map(|s| s.exit_code());
        // Son çıktının terminale ulaşması için okuyucuya kısa bir süre tanı.
        let _ = reader_finished.recv_timeout(Duration::from_millis(300));
        on_exit(code);
    });

    Ok(LocalTerm {
        writer,
        master: pair.master,
        killer,
    })
}

#[cfg(windows)]
fn default_shell() -> CommandBuilder {
    let mut c = CommandBuilder::new("powershell.exe");
    c.arg("-NoLogo");
    c
}

#[cfg(not(windows))]
fn default_shell() -> CommandBuilder {
    CommandBuilder::new_default_prog()
}

/// Menüde gösterilecek kabuklar.
pub fn available_shells() -> Vec<String> {
    #[cfg(windows)]
    {
        let mut v = vec!["powershell.exe".to_string(), "cmd.exe".to_string()];
        if which("pwsh.exe") {
            v.insert(0, "pwsh.exe".into());
        }
        if which("wsl.exe") {
            v.push("wsl.exe".into());
        }
        v
    }
    #[cfg(not(windows))]
    {
        let mut v: Vec<String> = std::fs::read_to_string("/etc/shells")
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with('/'))
            .filter(|l| std::path::Path::new(l).exists())
            .map(String::from)
            .collect();
        v.dedup();
        v
    }
}

#[cfg(windows)]
fn which(exe: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(exe).exists()))
        .unwrap_or(false)
}
