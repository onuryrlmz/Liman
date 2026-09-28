mod local;
mod sftp;
mod ssh;
mod store;

use std::{collections::HashMap, fmt::Display, path::PathBuf, sync::Arc, sync::Mutex};

use serde::{Deserialize, Serialize};
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    AppHandle, Emitter, State,
};

enum Term {
    Local(local::LocalTerm),
    Ssh(ssh::SshTerm),
}

#[derive(Default)]
struct AppState {
    terms: Mutex<HashMap<String, Term>>,
}

impl AppState {
    fn conn(&self, id: &str) -> Result<Arc<ssh::SshConn>, String> {
        match self.terms.lock().unwrap().get(id) {
            Some(Term::Ssh(t)) => Ok(t.conn.clone()),
            Some(Term::Local(_)) => Err("Bu sekme bir SSH oturumu değil".into()),
            None => Err("Oturum bulunamadı".into()),
        }
    }
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl Display) -> String {
    format!("{e:#}")
}

#[derive(Serialize, Clone)]
struct ExitPayload {
    id: String,
    code: Option<u32>,
}

fn sink(ch: Channel<InvokeResponseBody>) -> ssh::Sink {
    Arc::new(move |d| {
        let _ = ch.send(InvokeResponseBody::Raw(d));
    })
}

fn on_exit(app: AppHandle, id: String) -> ssh::OnExit {
    Box::new(move |code| {
        let _ = app.emit("term-exit", ExitPayload { id, code });
    })
}

fn reporter(app: AppHandle, id: String, name: String) -> sftp::Reporter {
    sftp::Reporter::new(id, name, move |p| {
        let _ = app.emit("transfer", p);
    })
}

// ---------- Terminaller ----------

#[tauri::command]
fn local_shells() -> Vec<String> {
    local::available_shells()
}

#[tauri::command]
fn local_spawn(
    app: AppHandle,
    state: State<'_, AppState>,
    shell: Option<String>,
    cols: u16,
    rows: u16,
    on_data: Channel<InvokeResponseBody>,
) -> CmdResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let term = local::spawn(shell, cols, rows, sink(on_data), on_exit(app, id.clone())).map_err(err)?;
    state
        .terms
        .lock()
        .unwrap()
        .insert(id.clone(), Term::Local(term));
    Ok(id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectRequest {
    session_id: Option<String>,
    host: String,
    port: u16,
    username: String,
    auth: store::AuthKind,
    key_path: Option<String>,
    secret: Option<String>,
    #[serde(default)]
    save_secret: bool,
}

#[tauri::command]
async fn ssh_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    req: ConnectRequest,
    cols: u32,
    rows: u32,
    on_data: Channel<InvokeResponseBody>,
) -> CmdResult<String> {
    let typed_secret = req.secret.clone().filter(|s| !s.is_empty());
    let secret = typed_secret.clone().or_else(|| {
        req.session_id
            .as_deref()
            .and_then(store::get_secret)
    });
    let params = ssh::ConnectParams {
        host: req.host.trim().to_string(),
        port: req.port,
        username: req.username.trim().to_string(),
        auth: req.auth,
        key_path: req.key_path,
        secret,
    };
    let id = uuid::Uuid::new_v4().to_string();
    let term = ssh::connect(params, cols, rows, sink(on_data), on_exit(app, id.clone()))
        .await
        .map_err(err)?;
    if let (true, Some(sid), Some(s)) = (req.save_secret, &req.session_id, &typed_secret) {
        if store::set_secret(sid, s).is_ok() {
            let _ = store::mark_secret_saved(sid);
        }
    }
    state.terms.lock().unwrap().insert(id.clone(), Term::Ssh(term));
    Ok(id)
}

#[tauri::command]
fn term_write(state: State<'_, AppState>, id: String, data: String) -> CmdResult<()> {
    match state.terms.lock().unwrap().get_mut(&id) {
        Some(Term::Local(t)) => t.write(data.as_bytes()).map_err(err),
        Some(Term::Ssh(t)) => t
            .tx
            .send(ssh::SshInput::Data(data.into_bytes()))
            .map_err(|_| "Bağlantı kapalı".to_string()),
        None => Ok(()),
    }
}

#[tauri::command]
fn term_resize(state: State<'_, AppState>, id: String, cols: u16, rows: u16) -> CmdResult<()> {
    match state.terms.lock().unwrap().get(&id) {
        Some(Term::Local(t)) => t.resize(cols, rows).map_err(err),
        Some(Term::Ssh(t)) => {
            let _ = t.tx.send(ssh::SshInput::Resize(cols as u32, rows as u32));
            Ok(())
        }
        None => Ok(()),
    }
}

#[tauri::command]
async fn term_close(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let term = state.terms.lock().unwrap().remove(&id);
    match term {
        Some(Term::Local(mut t)) => t.kill(),
        Some(Term::Ssh(t)) => {
            let _ = t.tx.send(ssh::SshInput::Close);
            t.conn.disconnect().await;
        }
        None => {}
    }
    Ok(())
}

// ---------- Oturumlar ----------

#[tauri::command]
fn sessions_list() -> CmdResult<Vec<store::Session>> {
    store::load_sessions().map_err(err)
}

#[tauri::command]
fn session_save(session: store::Session, secret: Option<String>) -> CmdResult<store::Session> {
    store::save_session(session, secret).map_err(err)
}

#[tauri::command]
fn session_delete(id: String) -> CmdResult<()> {
    store::delete_session(&id).map_err(err)
}

// ---------- SFTP ----------

#[tauri::command]
async fn sftp_home(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    let conn = state.conn(&id)?;
    let sftp = conn.sftp().await.map_err(err)?;
    sftp.canonicalize(".").await.map_err(err)
}

#[tauri::command]
async fn sftp_list(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> CmdResult<Vec<sftp::Entry>> {
    let conn = state.conn(&id)?;
    let s = conn.sftp().await.map_err(err)?;
    sftp::list(s, &path).await.map_err(err)
}

#[tauri::command]
async fn sftp_mkdir(state: State<'_, AppState>, id: String, path: String) -> CmdResult<()> {
    let conn = state.conn(&id)?;
    conn.sftp().await.map_err(err)?.create_dir(path).await.map_err(err)
}

#[tauri::command]
async fn sftp_rename(
    state: State<'_, AppState>,
    id: String,
    from: String,
    to: String,
) -> CmdResult<()> {
    let conn = state.conn(&id)?;
    conn.sftp().await.map_err(err)?.rename(from, to).await.map_err(err)
}

#[tauri::command]
async fn sftp_remove(
    state: State<'_, AppState>,
    id: String,
    path: String,
    is_dir: bool,
) -> CmdResult<()> {
    let conn = state.conn(&id)?;
    let s = conn.sftp().await.map_err(err)?;
    sftp::remove(s, path, is_dir).await.map_err(err)
}

#[tauri::command]
async fn sftp_read_text(state: State<'_, AppState>, id: String, path: String) -> CmdResult<String> {
    let conn = state.conn(&id)?;
    let s = conn.sftp().await.map_err(err)?;
    sftp::read_text(s, &path).await.map_err(err)
}

#[tauri::command]
async fn sftp_write_text(
    state: State<'_, AppState>,
    id: String,
    path: String,
    text: String,
) -> CmdResult<()> {
    let conn = state.conn(&id)?;
    let s = conn.sftp().await.map_err(err)?;
    sftp::write_text(s, &path, &text).await.map_err(err)
}

#[tauri::command]
async fn sftp_download(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    remote: String,
    local: String,
    transfer_id: String,
) -> CmdResult<()> {
    let conn = state.conn(&id)?;
    let name = remote.rsplit('/').next().unwrap_or(&remote).to_string();
    let mut rep = reporter(app, transfer_id, name);
    let res = match conn.sftp().await {
        Ok(s) => sftp::download(s, remote, PathBuf::from(local), &mut rep).await,
        Err(e) => Err(e),
    };
    let out = res.as_ref().map(|_| ()).map_err(err);
    rep.finish(&res);
    out
}

#[tauri::command]
async fn sftp_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    local: String,
    remote_dir: String,
    transfer_id: String,
) -> CmdResult<()> {
    let conn = state.conn(&id)?;
    let local = PathBuf::from(local);
    let mut rep = reporter(app, transfer_id, ssh::local_name(&local));
    let res = match conn.sftp().await {
        Ok(s) => sftp::upload(s, local, remote_dir, &mut rep).await,
        Err(e) => Err(e),
    };
    let out = res.as_ref().map(|_| ()).map_err(err);
    rep.finish(&res);
    out
}

// ---------- Tüneller ----------

#[tauri::command]
async fn tunnel_start(
    state: State<'_, AppState>,
    id: String,
    local_port: u16,
    remote_host: String,
    remote_port: u16,
) -> CmdResult<ssh::TunnelInfo> {
    let conn = state.conn(&id)?;
    conn.start_tunnel(local_port, remote_host, remote_port)
        .await
        .map_err(err)
}

#[tauri::command]
fn tunnel_stop(state: State<'_, AppState>, id: String, tunnel_id: String) -> CmdResult<()> {
    state.conn(&id)?.stop_tunnel(&tunnel_id);
    Ok(())
}

#[tauri::command]
fn tunnel_list(state: State<'_, AppState>, id: String) -> CmdResult<Vec<ssh::TunnelInfo>> {
    Ok(state.conn(&id)?.tunnels())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            local_shells,
            local_spawn,
            ssh_connect,
            term_write,
            term_resize,
            term_close,
            sessions_list,
            session_save,
            session_delete,
            sftp_home,
            sftp_list,
            sftp_mkdir,
            sftp_rename,
            sftp_remove,
            sftp_read_text,
            sftp_write_text,
            sftp_download,
            sftp_upload,
            tunnel_start,
            tunnel_stop,
            tunnel_list,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests;
