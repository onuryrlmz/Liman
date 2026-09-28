import { invoke, Channel } from "@tauri-apps/api/core";

export type AuthKind = "auto" | "password" | "key";
export type SessionKind = "ssh" | "sftp" | "telnet" | "serial" | "vnc" | "rdp";

export interface Session {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  auth: AuthKind;
  keyPath?: string | null;
  folder?: string | null;
  hasSecret?: boolean;
  kind?: SessionKind;
  /** Atlama sunucusu: kayıtlı oturum kimliği ya da "kullanıcı@sunucu:port". */
  jump?: string | null;
  /** Bağlanınca otomatik başlatılan tüneller. */
  tunnels?: TunnelSpec[];
  /** Seri port hızı (aygıt yolu `host` alanında). */
  baud?: number | null;
  /** SSH: sunucudaki grafik uygulamaları bu bilgisayarda aç. */
  x11?: boolean;
}

export interface ConnectRequest {
  sessionId?: string | null;
  host: string;
  port: number;
  username: string;
  auth: AuthKind;
  keyPath?: string | null;
  secret?: string | null;
  saveSecret?: boolean;
  /** Kabuk açmadan yalnızca SFTP. */
  sftpOnly?: boolean;
  jump?: string | null;
  x11?: boolean;
  /** Kabuk ve SFTP olmadan, yalnızca tünel için (VNC/RDP). */
  tunnelOnly?: boolean;
}

export interface Snippet {
  id: string;
  name: string;
  command: string;
  /** Gönderdikten sonra Enter'a basılsın mı? */
  run: boolean;
}

export interface HostKeyQuestion {
  id: string;
  host: string;
  port: number;
  algorithm: string;
  fingerprint: string;
  changed: boolean;
}

export interface ImportPreview {
  sessions: Session[];
  unsupported: number;
}

export interface Entry {
  name: string;
  path: string;
  isDir: boolean;
  isLink: boolean;
  size: number;
  mtime: number | null;
  perms: string;
}

export type TunnelKind = "local" | "remote" | "socks";

/** Kaydedilebilir tünel tanımı. */
export interface TunnelSpec {
  kind: TunnelKind;
  listenHost: string;
  listenPort: number;
  targetHost: string;
  targetPort: number;
}

export interface TunnelInfo extends TunnelSpec {
  id: string;
}

export interface Transfer {
  id: string;
  name: string;
  done: number;
  total: number;
  state: "run" | "done" | "error";
  error?: string | null;
  direction?: "up" | "down";
}

export interface ExportResult {
  sessions: number;
  secrets: number;
  keys: number;
}

export interface ImportResult {
  added: number;
  updated: number;
  secrets: number;
  keys: number;
  groups: number;
  snippets?: number;
}

export interface BackupFileInfo {
  encrypted: boolean;
  sessions: number | null;
}

export const AUTH_FAILED = "AUTH_FAILED";
export const HOST_KEY_REJECTED = "HOST_KEY_REJECTED";
export const PASSWORD_REQUIRED = "PASSWORD_REQUIRED";
export const WRONG_PASSWORD = "WRONG_PASSWORD";

function dataChannel(onData: (d: Uint8Array) => void) {
  const ch = new Channel<ArrayBuffer>();
  ch.onmessage = (m) => onData(new Uint8Array(m));
  return ch;
}

export const api = {
  localShells: () => invoke<string[]>("local_shells"),
  localSpawn: (shell: string | null, cols: number, rows: number, onData: (d: Uint8Array) => void, logName: string) =>
    invoke<string>("local_spawn", { shell, cols, rows, onData: dataChannel(onData), logName }),
  sshConnect: (req: ConnectRequest, cols: number, rows: number, onData: (d: Uint8Array) => void, logName: string) =>
    invoke<string>("ssh_connect", { req, cols, rows, onData: dataChannel(onData), logName }),
  sshShare: (source: string, cols: number, rows: number, onData: (d: Uint8Array) => void, logName: string) =>
    invoke<string>("ssh_share", { source, cols, rows, onData: dataChannel(onData), logName }),
  localList: (path: string) => invoke<{ path: string; parent: string | null; entries: Entry[] }>("local_list", { path }),
  localMkdir: (parent: string, name: string) => invoke<void>("local_mkdir", { parent, name }),
  localRename: (path: string, name: string) => invoke<void>("local_rename", { path, name }),
  localRemove: (path: string) => invoke<void>("local_remove", { path }),
  logDefaultDir: () => invoke<string>("log_default_dir"),
  termWrite: (id: string, data: string) => invoke<void>("term_write", { id, data }),
  termResize: (id: string, cols: number, rows: number) => invoke<void>("term_resize", { id, cols, rows }),
  termClose: (id: string) => invoke<void>("term_close", { id }),

  sessionsList: () => invoke<Session[]>("sessions_list"),
  sessionSave: (session: Session, secret: string | null) => invoke<Session>("session_save", { session, secret }),
  sessionDelete: (id: string) => invoke<void>("session_delete", { id }),
  sessionMove: (id: string, folder: string | null) => invoke<void>("session_move", { id, folder }),
  groupsList: () => invoke<string[]>("groups_list"),
  groupCreate: (name: string) => invoke<void>("group_create", { name }),
  groupRename: (old: string, name: string) => invoke<void>("group_rename", { old, new: name }),
  groupDelete: (name: string, deleteSessions: boolean) => invoke<void>("group_delete", { name, deleteSessions }),
  sessionsExport: (path: string, password: string | null, group: string | null) =>
    invoke<ExportResult>("sessions_export", { path, password, group }),
  sessionsImportInfo: (path: string) => invoke<BackupFileInfo>("sessions_import_info", { path }),
  telnetConnect: (host: string, port: number, cols: number, rows: number, onData: (d: Uint8Array) => void, logName: string) =>
    invoke<string>("telnet_connect", { host, port, cols, rows, onData: dataChannel(onData), logName }),
  serialPorts: () => invoke<{ path: string; description: string }[]>("serial_ports"),
  serialConnect: (path: string, baud: number, onData: (d: Uint8Array) => void, logName: string) =>
    invoke<string>("serial_connect", { path, baud, onData: dataChannel(onData), logName }),
  streamOpen: (host: string, port: number, via: string | null, onData: (d: Uint8Array) => void) =>
    invoke<string>("stream_open", { host, port, via, onData: dataChannel(onData) }),
  streamWrite: (id: string, data: Uint8Array) => invoke<void>("stream_write", data, { headers: { "x-stream": id } }),
  streamClose: (id: string) => invoke<void>("stream_close", { id }),
  sessionPassword: (sessionId: string) => invoke<string | null>("session_password", { sessionId }),
  rdpPrepare: (host: string, port: number, username: string, via: string | null) =>
    invoke<[string, string]>("rdp_prepare", { host, port, username, via }),
  hostKeyAnswer: (id: string, accept: boolean) => invoke<void>("host_key_answer", { id, accept }),
  importPreview: (source: "ssh-config" | "mobaxterm", path: string | null) =>
    invoke<ImportPreview>("import_preview", { source, path }),
  importSessions: (sessions: Session[]) => invoke<{ added: number; skipped: number }>("import_sessions", { sessions }),
  snippetsList: () => invoke<Snippet[]>("snippets_list"),
  snippetsSave: (snippets: Snippet[]) => invoke<void>("snippets_save", { snippets }),
  settingsGet: () => invoke<Record<string, unknown>>("settings_get"),
  settingsSet: (value: Record<string, unknown>) => invoke<void>("settings_set", { value }),
  sessionsImport: (path: string, password: string | null) => invoke<ImportResult>("sessions_import", { path, password }),

  sftpHome: (id: string) => invoke<string>("sftp_home", { id }),
  sftpList: (id: string, path: string) => invoke<Entry[]>("sftp_list", { id, path }),
  sftpMkdir: (id: string, path: string) => invoke<void>("sftp_mkdir", { id, path }),
  sftpRename: (id: string, from: string, to: string) => invoke<void>("sftp_rename", { id, from, to }),
  sftpRemove: (id: string, path: string, isDir: boolean) => invoke<void>("sftp_remove", { id, path, isDir }),
  sftpReadText: (id: string, path: string) => invoke<string>("sftp_read_text", { id, path }),
  sftpWriteText: (id: string, path: string, text: string) => invoke<void>("sftp_write_text", { id, path, text }),
  syncCompare: (id: string, local: string, remote: string) =>
    invoke<{ diffs: unknown[]; same: number; truncated: boolean }>("sync_compare", { id, local, remote }),
  syncApply: (id: string, local: string, remote: string, actions: { path: string; direction: "upload" | "download" }[]) =>
    invoke<number>("sync_apply", { id, local, remote, actions }),
  sftpEditLocal: (id: string, remote: string) => invoke<string>("sftp_edit_local", { id, remote }),
  sftpDownload: (id: string, remote: string, local: string, transferId: string) =>
    invoke<void>("sftp_download", { id, remote, local, transferId }),
  sftpUpload: (id: string, local: string, remoteDir: string, transferId: string) =>
    invoke<void>("sftp_upload", { id, local, remoteDir, transferId }),

  tunnelStart: (id: string, t: TunnelSpec) => invoke<TunnelInfo>("tunnel_start", { id, ...t }),
  tunnelStop: (id: string, tunnelId: string) => invoke<void>("tunnel_stop", { id, tunnelId }),
  tunnelList: (id: string) => invoke<TunnelInfo[]>("tunnel_list", { id }),
};

export function joinPath(dir: string, name: string) {
  return dir.endsWith("/") ? dir + name : `${dir}/${name}`;
}

export function parentPath(p: string) {
  if (p === "/" || !p.includes("/")) return "/";
  const up = p.replace(/\/+$/, "").split("/").slice(0, -1).join("/");
  return up || "/";
}

export function formatSize(n: number) {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

export function formatDate(secs: number | null) {
  if (!secs) return "";
  const d = new Date(secs * 1000);
  const pad = (x: number) => String(x).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** "[ssh|sftp|telnet://]kullanici@host:port" biçimini çözer. */
export function parseQuick(
  s: string,
): { username: string; host: string; port: number; sftpOnly: boolean; telnet: boolean } | null {
  const m = s.trim().match(/^(?:(ssh|sftp|telnet):\/\/)?(?:([^@\s]+)@)?([^:\s/]+)(?::(\d+))?\/?$/i);
  if (!m) return null;
  const scheme = m[1]?.toLowerCase();
  return {
    username: m[2] ?? "",
    host: m[3],
    port: m[4] ? Number(m[4]) : scheme === "telnet" ? 23 : 22,
    sftpOnly: scheme === "sftp",
    telnet: scheme === "telnet",
  };
}

/** Oturum türü için simge ve kısa etiket. */
export const kindInfo: Record<SessionKind, { icon: string; badge: string | null; label: string }> = {
  ssh: { icon: "server", badge: null, label: "SSH" },
  sftp: { icon: "folder", badge: "SFTP", label: "SFTP" },
  telnet: { icon: "terminal", badge: "TELNET", label: "Telnet" },
  serial: { icon: "plug", badge: "SERİ", label: "Seri port" },
  vnc: { icon: "monitor", badge: "VNC", label: "VNC" },
  rdp: { icon: "monitor", badge: "RDP", label: "RDP" },
};

export function errText(e: unknown) {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
