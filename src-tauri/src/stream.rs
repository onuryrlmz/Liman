//! Ham bayt akışları: VNC gibi arayüzde çalışan protokoller için doğrudan TCP ya da
//! açık bir SSH bağlantısı üzerinden tünel.

use std::{collections::HashMap, sync::Mutex, time::Duration};

use anyhow::{anyhow, Context, Result};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc,
};

use crate::ssh::{OnExit, Sink, SshConn};

#[derive(Default)]
pub struct Streams {
    map: Mutex<HashMap<String, mpsc::UnboundedSender<Option<Vec<u8>>>>>,
}

impl Streams {
    pub fn write(&self, id: &str, data: Vec<u8>) -> Result<()> {
        let map = self.map.lock().unwrap();
        let tx = map.get(id).ok_or_else(|| anyhow!("Akış kapalı"))?;
        tx.send(Some(data)).map_err(|_| anyhow!("Akış kapalı"))
    }

    pub fn close(&self, id: &str) {
        if let Some(tx) = self.map.lock().unwrap().remove(id) {
            let _ = tx.send(None);
        }
    }

    fn pump<S>(&self, id: String, stream: S, out: Sink, on_exit: OnExit)
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (mut rd, mut wr) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::unbounded_channel::<Option<Vec<u8>>>();
        self.map.lock().unwrap().insert(id, tx);
        tokio::spawn(async move {
            while let Some(Some(buf)) = rx.recv().await {
                if wr.write_all(&buf).await.is_err() {
                    break;
                }
            }
            let _ = wr.shutdown().await;
        });
        tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match rd.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => out(buf[..n].to_vec()),
                }
            }
            on_exit(None);
        });
    }

    /// `via` verilirse bağlantı o SSH bağlantısı üzerinden (sunucunun gözünden) kurulur.
    pub async fn open(
        &self,
        id: String,
        host: &str,
        port: u16,
        via: Option<&SshConn>,
        out: Sink,
        on_exit: OnExit,
    ) -> Result<()> {
        match via {
            Some(conn) => {
                let ch = conn
                    .direct_tcpip(host, port)
                    .await
                    .with_context(|| format!("SSH üzerinden {host}:{port} adresine ulaşılamadı"))?;
                self.pump(id, ch, out, on_exit);
            }
            None => {
                let sock = tokio::time::timeout(Duration::from_secs(15), tokio::net::TcpStream::connect((host, port)))
                    .await
                    .map_err(|_| anyhow!("{host}:{port} bağlantısı zaman aşımına uğradı"))?
                    .with_context(|| format!("{host}:{port} adresine bağlanılamadı"))?;
                let _ = sock.set_nodelay(true);
                self.pump(id, sock, out, on_exit);
            }
        }
        Ok(())
    }
}
