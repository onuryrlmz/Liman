mod backup;
mod importers;
mod local;
mod logger;
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
    /// Cevap bekleyen sunucu anahtarı soruları.
    host_key_questions: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
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

/// Terminal çıktısı: arayüze gider, oturum kaydı açıksa dosyaya da yazılır.
fn sink(ch: Channel<InvokeResponseBody>, log_name: Option<&str>) -> ssh::Sink {
    let log = log_name.and_then(logger::SessionLog::open);
    Arc::new(move |d| {
        if let Some(l) = &log {
            l.write(&d);
        }
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
    log_name: Option<String>,
) -> CmdResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let out = sink(on_data, log_name.as_deref());
    let term = local::spawn(shell, cols, rows, out, on_exit(app, id.clone())).map_err(err)?;
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
    #[serde(default)]
    sftp_only: bool,
    #[serde(default)]
    jump: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct HostKeyEvent {
    id: String,
    #[serde(flatten)]
    question: ssh::HostKeyQuestion,
}

fn host_key_asker(app: AppHandle, pending: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>) -> ssh::AskHostKey {
    Arc::new(move |question| {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let id = uuid::Uuid::new_v4().to_string();
        pending.lock().unwrap().insert(id.clone(), tx);
        let _ = app.emit("host-key", HostKeyEvent { id, question });
        rx
    })
}

#[tauri::command]
fn host_key_answer(state: State<'_, AppState>, id: String, accept: bool) {
    if let Some(tx) = state.host_key_questions.lock().unwrap().remove(&id) {
        let _ = tx.send(accept);
    }
}

/// "kullanıcı@sunucu:port" ya da kayıtlı oturum kimliğinden atlama sunucusunu çözer.
fn resolve_jump(spec: &str, depth: usize) -> Result<ssh::ConnectParams, String> {
    if depth > 5 {
        return Err("Atlama sunucusu zinciri çok uzun (döngü olabilir)".into());
    }
    // "a,b" zinciri: önce a'ya, onun üzerinden b'ye bağlanılır.
    if let Some((first, last)) = spec.rsplit_once(',') {
        let mut p = resolve_jump(last.trim(), depth + 1)?;
        p.jump = Some(Box::new(resolve_jump(first.trim(), depth + 1)?));
        return Ok(p);
    }
    let sessions = store::load_sessions().map_err(err)?;
    if let Some(s) = sessions.iter().find(|s| s.id == spec) {
        return Ok(ssh::ConnectParams {
            host: s.host.clone(),
            port: s.port,
            username: s.username.clone(),
            auth: s.auth,
            key_path: s.key_path.clone(),
            secret: store::get_secret(&s.id),
            shell: false,
            jump: match s.jump.as_deref().filter(|j| !j.is_empty()) {
                Some(j) => Some(Box::new(resolve_jump(j, depth + 1)?)),
                None => None,
            },
        });
    }
    let (user, rest) = match spec.rsplit_once('@') {
        Some((u, r)) => (u.to_string(), r),
        None => (whoami(), spec),
    };
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().map_err(|_| format!("Geçersiz port: {p}"))?),
        None => (rest, 22),
    };
    if host.is_empty() {
        return Err(format!("Geçersiz atlama sunucusu: {spec}"));
    }
    Ok(ssh::ConnectParams {
        host: host.to_string(),
        port,
        username: user,
        auth: store::AuthKind::Auto,
        key_path: None,
        secret: None,
        shell: false,
        jump: None,
    })
}

fn whoami() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

#[tauri::command]
async fn ssh_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    req: ConnectRequest,
    cols: u32,
    rows: u32,
    on_data: Channel<InvokeResponseBody>,
    log_name: Option<String>,
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
        shell: !req.sftp_only,
        jump: match req.jump.as_deref().map(str::trim).filter(|j| !j.is_empty()) {
            Some(j) => Some(Box::new(resolve_jump(j, 0)?)),
            None => None,
        },
    };
    let id = uuid::Uuid::new_v4().to_string();
    let ask = host_key_asker(app.clone(), state.host_key_questions.clone());
    let out = sink(on_data, log_name.as_deref());
    let term = ssh::connect(params, cols, rows, out, ask, on_exit(app, id.clone()))
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

/// Açık bir SSH bağlantısı üzerinde yeni bir kabuk açar (bölünmüş pano).
#[tauri::command]
async fn ssh_share(
    app: AppHandle,
    state: State<'_, AppState>,
    source: String,
    cols: u32,
    rows: u32,
    on_data: Channel<InvokeResponseBody>,
    log_name: Option<String>,
) -> CmdResult<String> {
    let conn = state.conn(&source)?;
    if conn.is_closed() {
        return Err("Kaynak bağlantı kapalı".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let term = conn
        .open_shell(cols, rows, sink(on_data, log_name.as_deref()), on_exit(app, id.clone()))
        .await
        .map_err(err)?;
    state.terms.lock().unwrap().insert(id.clone(), Term::Ssh(term));
    Ok(id)
}

#[tauri::command]
fn term_write(state: State<'_, AppState>, id: String, data: String) -> CmdResult<()> {
    match state.terms.lock().unwrap().get_mut(&id) {
        Some(Term::Local(t)) => t.write(data.as_bytes()).map_err(err),
        Some(Term::Ssh(t)) => t.send(ssh::SshInput::Data(data.into_bytes())).map_err(err),
        None => Ok(()),
    }
}

#[tauri::command]
fn term_resize(state: State<'_, AppState>, id: String, cols: u16, rows: u16) -> CmdResult<()> {
    match state.terms.lock().unwrap().get(&id) {
        Some(Term::Local(t)) => t.resize(cols, rows).map_err(err),
        Some(Term::Ssh(t)) => {
            let _ = t.send(ssh::SshInput::Resize(cols as u32, rows as u32));
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
            let _ = t.send(ssh::SshInput::Close);
            // Bağlantıyı paylaşan başka pano yoksa kapat.
            let shared = state
                .terms
                .lock()
                .unwrap()
                .values()
                .any(|x| matches!(x, Term::Ssh(o) if Arc::ptr_eq(&o.conn, &t.conn)));
            if !shared {
                t.conn.disconnect().await;
            }
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

#[tauri::command]
fn session_move(id: String, folder: Option<String>) -> CmdResult<()> {
    store::move_session(&id, folder).map_err(err)
}

#[tauri::command]
fn groups_list() -> CmdResult<Vec<String>> {
    store::load_groups().map_err(err)
}

#[tauri::command]
fn group_create(name: String) -> CmdResult<()> {
    store::create_group(&name).map_err(err)
}

#[tauri::command]
fn group_rename(old: String, new: String) -> CmdResult<()> {
    store::rename_group(&old, &new).map_err(err)
}

#[tauri::command]
fn group_delete(name: String, delete_sessions: bool) -> CmdResult<()> {
    store::delete_group(&name, delete_sessions).map_err(err)
}

// Argon2 bilinçli olarak yavaş; arayüzü kilitlememesi için ayrı iş parçacığında.
#[tauri::command]
async fn sessions_export(
    path: String,
    password: Option<String>,
    group: Option<String>,
) -> CmdResult<backup::ExportResult> {
    tauri::async_runtime::spawn_blocking(move || {
        backup::export(PathBuf::from(path).as_path(), password.as_deref(), group.as_deref())
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
fn log_default_dir() -> String {
    logger::default_dir().to_string_lossy().into_owned()
}

#[tauri::command]
fn snippets_list() -> Vec<store::Snippet> {
    store::load_snippets()
}

#[tauri::command]
fn snippets_save(snippets: Vec<store::Snippet>) -> CmdResult<()> {
    store::save_snippets(&snippets).map_err(err)
}

#[tauri::command]
fn settings_get() -> serde_json::Value {
    store::load_settings()
}

#[tauri::command]
fn settings_set(value: serde_json::Value) -> CmdResult<()> {
    store::save_settings(&value).map_err(err)
}

#[tauri::command]
fn import_preview(source: String, path: Option<String>) -> CmdResult<importers::Preview> {
    importers::preview(&source, path.as_deref().map(std::path::Path::new)).map_err(err)
}

#[tauri::command]
fn import_sessions(sessions: Vec<store::Session>) -> CmdResult<importers::Imported> {
    importers::import(sessions).map_err(err)
}

#[tauri::command]
fn sessions_import_info(path: String) -> CmdResult<backup::FileInfo> {
    backup::inspect(PathBuf::from(path).as_path()).map_err(err)
}

#[tauri::command]
async fn sessions_import(path: String, password: Option<String>) -> CmdResult<backup::ImportResult> {
    tauri::async_runtime::spawn_blocking(move || {
        backup::import(PathBuf::from(path).as_path(), password.as_deref())
    })
    .await
    .map_err(err)?
    .map_err(err)
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            local_shells,
            local_spawn,
            ssh_connect,
            host_key_answer,
            ssh_share,
            term_write,
            term_resize,
            term_close,
            sessions_list,
            session_save,
            session_delete,
            session_move,
            groups_list,
            group_create,
            group_rename,
            group_delete,
            sessions_export,
            sessions_import_info,
            sessions_import,
            import_preview,
            settings_get,
            snippets_list,
            log_default_dir,
            snippets_save,
            settings_set,
            import_sessions,
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
