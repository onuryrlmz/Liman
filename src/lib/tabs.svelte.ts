import { api, errText, type ConnectRequest, type Session, type Snippet, type Transfer } from "./api";

export type TabStatus = "connecting" | "open" | "closed" | "error";

/** Tek bir terminal (yerel kabuk, SSH kabuğu ya da yalnızca SFTP). */
export interface Pane {
  key: string;
  /** telnet: connect.host/port; serial: connect.host = aygıt, connect.port = baud. */
  kind: "local" | "ssh" | "sftp" | "telnet" | "serial" | "vnc" | "rdp";
  title: string;
  status: TabStatus;
  termId: string | null;
  shell?: string | null;
  connect?: ConnectRequest;
  sessionId?: string | null;
  hasSecret?: boolean;
  remoteTitle?: string;
  /** Bölünmüş pano: bu açık SSH bağlantısı üzerinde yeni kabuk açılır. */
  shareFrom?: string | null;
}

/** Eski adı; terminal bileşeni bir panoyu temsil eder. */
export type Tab = Pane;
export type TabPatch = Partial<Omit<Pane, "key" | "kind">>;

export type SplitDir = "row" | "col";
export type Layout =
  | { type: "pane"; pane: string }
  | { type: "split"; dir: SplitDir; ratio: number; a: Layout; b: Layout };

/** Sekme çubuğundaki bir sekme: bir ya da daha fazla pano. */
export interface View {
  key: string;
  root: Layout;
  focus: string;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

let counter = 0;
const nextKey = (prefix: string) => `${prefix}-${++counter}`;

// ---------- Yerleşim ağacı ----------

export function leaves(node: Layout): string[] {
  return node.type === "pane" ? [node.pane] : [...leaves(node.a), ...leaves(node.b)];
}

function splitLeaf(node: Layout, target: string, dir: SplitDir, added: string): Layout {
  if (node.type === "pane") {
    return node.pane === target
      ? { type: "split", dir, ratio: 0.5, a: node, b: { type: "pane", pane: added } }
      : node;
  }
  return { ...node, a: splitLeaf(node.a, target, dir, added), b: splitLeaf(node.b, target, dir, added) };
}

function removeLeaf(node: Layout, target: string): Layout | null {
  if (node.type === "pane") return node.pane === target ? null : node;
  const a = removeLeaf(node.a, target);
  const b = removeLeaf(node.b, target);
  if (!a) return b;
  if (!b) return a;
  return { ...node, a, b };
}

/** Panoların ve bölücülerin konumları (0–1 oranında). */
export function computeLayout(node: Layout, r: Rect = { x: 0, y: 0, w: 1, h: 1 }) {
  const panes = new Map<string, Rect>();
  const dividers: { node: Extract<Layout, { type: "split" }>; area: Rect }[] = [];
  const walk = (n: Layout, rect: Rect) => {
    if (n.type === "pane") {
      panes.set(n.pane, rect);
      return;
    }
    dividers.push({ node: n, area: rect });
    if (n.dir === "row") {
      const w = rect.w * n.ratio;
      walk(n.a, { ...rect, w });
      walk(n.b, { ...rect, x: rect.x + w, w: rect.w - w });
    } else {
      const h = rect.h * n.ratio;
      walk(n.a, { ...rect, h });
      walk(n.b, { ...rect, y: rect.y + h, h: rect.h - h });
    }
  };
  walk(node, r);
  return { panes, dividers };
}

class Store {
  panes = $state<Pane[]>([]);
  views = $state<View[]>([]);
  activeKey = $state<string | null>(null);
  sessions = $state<Session[]>([]);
  groups = $state<string[]>([]);
  shells = $state<string[]>([]);
  transfers = $state<Transfer[]>([]);
  /** SSH sekmesi başına SFTP klasörü (termId → yol). */
  sftpPaths = $state<Record<string, string>>({});
  toast = $state<{ text: string; kind: "info" | "error" } | null>(null);
  /** MultiExec: yazılanlar etkin sekmenin tüm panolarına ya da tüm sekmelere de gider. */
  broadcast = $state<"off" | "tab" | "all">("off");
  snippets = $state<Snippet[]>([]);
  /** Arttıkça odaktaki terminal klavye odağını geri alır. */
  focusRequest = $state(0);

  async loadSnippets() {
    try {
      this.snippets = await api.snippetsList();
    } catch (e) {
      this.notify(errText(e), "error");
    }
  }

  async saveSnippets(list: Snippet[]) {
    this.snippets = list;
    try {
      await api.snippetsSave(list);
    } catch (e) {
      this.notify(errText(e), "error");
    }
  }

  /** Metni odaktaki terminale (MultiExec açıksa diğerlerine de) yazar. */
  sendText(text: string) {
    const p = this.active;
    if (!p || p.kind === "sftp" || !p.termId || p.status !== "open") {
      this.notify("Göndermek için bağlı bir terminal seçin", "error");
      return;
    }
    for (const id of [p.termId, ...this.broadcastTargets(p.key)]) api.termWrite(id, text);
    this.focusRequest++;
  }

  runSnippet(s: Snippet) {
    this.sendText(s.command.replace(/\r?\n/g, "\r") + (s.run ? "\r" : ""));
  }

  /** `paneKey` panosunda yazılanın ayrıca gönderileceği terminaller. */
  broadcastTargets(paneKey: string): string[] {
    if (this.broadcast === "off") return [];
    const scope =
      this.broadcast === "all"
        ? this.panes
        : leaves(this.viewOf(paneKey)?.root ?? { type: "pane", pane: paneKey }).map((k) => this.pane(k));
    return scope
      .filter((p): p is Pane => !!p && p.key !== paneKey && p.kind !== "sftp" && p.status === "open" && !!p.termId)
      .map((p) => p.termId!);
  }

  /** MultiExec'ten etkilenen pano mu? (uyarı çerçevesi için) */
  inBroadcast(paneKey: string): boolean {
    if (this.broadcast === "off") return false;
    if (this.broadcast === "all") return true;
    return this.viewOf(paneKey)?.key === this.activeKey;
  }

  get activeView(): View | null {
    return this.views.find((v) => v.key === this.activeKey) ?? null;
  }

  /** Etkin sekmenin odaktaki panosu. */
  get active(): Pane | null {
    const v = this.activeView;
    return v ? this.pane(v.focus) : null;
  }

  pane(key: string): Pane | null {
    return this.panes.find((p) => p.key === key) ?? null;
  }

  viewOf(paneKey: string): View | null {
    return this.views.find((v) => leaves(v.root).includes(paneKey)) ?? null;
  }

  async loadSessions() {
    try {
      [this.sessions, this.groups] = await Promise.all([api.sessionsList(), api.groupsList()]);
    } catch (e) {
      this.notify(errText(e), "error");
    }
  }

  private addView(pane: Pane) {
    this.panes.push(pane);
    const view: View = { key: nextKey("view"), root: { type: "pane", pane: pane.key }, focus: pane.key };
    this.views.push(view);
    this.activeKey = view.key;
  }

  private localPane(shell: string | null): Pane {
    const title = shell ? (shell.split(/[\\/]/).pop() ?? shell) : "Yerel terminal";
    return { key: nextKey("pane"), kind: "local", title, status: "connecting", termId: null, shell };
  }

  openLocal(shell: string | null = null) {
    this.addView(this.localPane(shell));
  }

  private sshPane(connect: ConnectRequest, title: string, session?: Session, kind?: Pane["kind"]): Pane {
    return {
      key: nextKey("pane"),
      kind: kind ?? (connect.sftpOnly ? "sftp" : "ssh"),
      title,
      status: "connecting",
      termId: null,
      connect: { ...connect },
      sessionId: session?.id ?? null,
      hasSecret: session?.hasSecret ?? false,
    };
  }

  openSsh(connect: ConnectRequest, title: string, session?: Session, kind?: Pane["kind"]) {
    this.addView(this.sshPane(connect, title, session, kind));
  }

  /** `sftpOnly` verilmezse oturumun kendi türü kullanılır. */
  openSession(s: Session, sftpOnly = s.kind === "sftp") {
    if (s.kind === "telnet" || s.kind === "serial") {
      const port = s.kind === "serial" ? (s.baud ?? 115200) : s.port;
      return this.openSsh({ sessionId: s.id, host: s.host, port, username: "", auth: "auto" }, s.name || s.host, s, s.kind);
    }
    if (s.kind === "vnc" || s.kind === "rdp") {
      // jump: tünelin kurulacağı SSH sunucusu.
      return this.openSsh(
        { sessionId: s.id, host: s.host, port: s.port, username: s.username, auth: "auto", jump: s.jump },
        s.name || s.host,
        s,
        s.kind,
      );
    }
    this.openSsh(
      {
        sessionId: s.id,
        host: s.host,
        port: s.port,
        username: s.username,
        auth: s.auth,
        keyPath: s.keyPath,
        sftpOnly,
        jump: s.jump,
        x11: s.x11,
      },
      s.name || s.host,
      s,
    );
  }

  /** Aynı bağlantının/kabuğun yeni bir panosu (sekme ya da bölme için). */
  private clonePane(p: Pane, share: boolean): Pane {
    if (p.kind === "local") return this.localPane(p.shell ?? null);
    const s = this.sessions.find((x) => x.id === p.sessionId);
    if (p.kind !== "ssh" && p.kind !== "sftp") return this.sshPane({ ...p.connect! }, p.title, s, p.kind);
    // Bölmede SFTP panosundan da terminal açılır; aynı bağlantı paylaşılır.
    const pane = this.sshPane({ ...p.connect!, sftpOnly: share ? false : p.connect!.sftpOnly }, p.title, s);
    if (share && p.termId && p.status === "open") pane.shareFrom = p.termId;
    return pane;
  }

  duplicate(p: Pane) {
    this.addView(this.clonePane(p, false));
  }

  /** Odaktaki panoyu böler; yeni pano aynı sunucuya (aynı bağlantı üzerinden) açılır. */
  split(dir: SplitDir, view = this.activeView) {
    if (!view) return;
    const src = this.pane(view.focus);
    if (!src) return;
    const pane = this.clonePane(src, true);
    this.panes.push(pane);
    view.root = splitLeaf(view.root, src.key, dir, pane.key);
    view.focus = pane.key;
  }

  focusPane(paneKey: string) {
    const v = this.viewOf(paneKey);
    if (!v) return;
    v.focus = paneKey;
    this.activeKey = v.key;
  }

  focusNext(dir: 1 | -1) {
    const v = this.activeView;
    if (!v) return;
    const list = leaves(v.root);
    const i = list.indexOf(v.focus);
    v.focus = list[(i + dir + list.length) % list.length];
  }

  patch(key: string, p: TabPatch) {
    const t = this.pane(key);
    if (!t) return;
    Object.assign(t, p);
    if (p.hasSecret && t.sessionId) {
      const s = this.sessions.find((x) => x.id === t.sessionId);
      if (s) s.hasSecret = true;
    }
  }

  /** Panoyu kapatır; sekmedeki son panoysa sekmeyi de. */
  closePane(paneKey: string) {
    const v = this.viewOf(paneKey);
    if (!v) return;
    const root = removeLeaf(v.root, paneKey);
    if (!root) return this.close(v.key);
    // Terminal bileşeni kaldırılırken arka taraftaki oturumu da kapatır.
    this.panes = this.panes.filter((p) => p.key !== paneKey);
    v.root = root;
    if (v.focus === paneKey) v.focus = leaves(root)[0];
  }

  /** Sekmeyi tüm panolarıyla kapatır. */
  close(viewKey: string) {
    const i = this.views.findIndex((v) => v.key === viewKey);
    if (i < 0) return;
    const keys = new Set(leaves(this.views[i].root));
    this.views.splice(i, 1);
    this.panes = this.panes.filter((p) => !keys.has(p.key));
    if (this.activeKey === viewKey) {
      this.activeKey = this.views[Math.min(i, this.views.length - 1)]?.key ?? null;
    }
  }

  cycle(dir: 1 | -1) {
    if (!this.views.length) return;
    const i = this.views.findIndex((v) => v.key === this.activeKey);
    this.activeKey = this.views[(i + dir + this.views.length) % this.views.length].key;
  }

  upsertTransfer(t: Transfer) {
    const cur = this.transfers.find((x) => x.id === t.id);
    if (cur) Object.assign(cur, t);
    else this.transfers.push(t);
    if (t.state !== "run") {
      setTimeout(() => {
        this.transfers = this.transfers.filter((x) => x.id !== t.id || x.state === "run");
      }, t.state === "error" ? 8000 : 3000);
    }
  }

  private toastTimer: ReturnType<typeof setTimeout> | undefined;
  notify(text: string, kind: "info" | "error" = "info") {
    this.toast = { text, kind };
    clearTimeout(this.toastTimer);
    this.toastTimer = setTimeout(() => (this.toast = null), kind === "error" ? 6000 : 2500);
  }
}

export const store = new Store();
