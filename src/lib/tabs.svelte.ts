import { api, errText, type ConnectRequest, type Session, type Transfer } from "./api";

export type TabStatus = "connecting" | "open" | "closed" | "error";

export interface Tab {
  key: string;
  kind: "local" | "ssh";
  title: string;
  status: TabStatus;
  termId: string | null;
  shell?: string | null;
  connect?: ConnectRequest;
  sessionId?: string | null;
  hasSecret?: boolean;
  remoteTitle?: string;
}

export type TabPatch = Partial<Omit<Tab, "key" | "kind">>;

let counter = 0;
const nextKey = () => `tab-${++counter}`;

class Store {
  tabs = $state<Tab[]>([]);
  activeKey = $state<string | null>(null);
  sessions = $state<Session[]>([]);
  shells = $state<string[]>([]);
  transfers = $state<Transfer[]>([]);
  /** SSH sekmesi başına SFTP klasörü (termId → yol). */
  sftpPaths = $state<Record<string, string>>({});
  toast = $state<{ text: string; kind: "info" | "error" } | null>(null);

  get active(): Tab | null {
    return this.tabs.find((t) => t.key === this.activeKey) ?? null;
  }

  async loadSessions() {
    try {
      this.sessions = await api.sessionsList();
    } catch (e) {
      this.notify(errText(e), "error");
    }
  }

  openLocal(shell: string | null = null) {
    const name = shell ? (shell.split(/[\\/]/).pop() ?? shell) : "Yerel terminal";
    const tab: Tab = { key: nextKey(), kind: "local", title: name, status: "connecting", termId: null, shell };
    this.tabs.push(tab);
    this.activeKey = tab.key;
  }

  openSsh(connect: ConnectRequest, title: string, session?: Session) {
    const tab: Tab = {
      key: nextKey(),
      kind: "ssh",
      title,
      status: "connecting",
      termId: null,
      connect: { ...connect },
      sessionId: session?.id ?? null,
      hasSecret: session?.hasSecret ?? false,
    };
    this.tabs.push(tab);
    this.activeKey = tab.key;
  }

  openSession(s: Session) {
    this.openSsh(
      { sessionId: s.id, host: s.host, port: s.port, username: s.username, auth: s.auth, keyPath: s.keyPath },
      s.name || s.host,
      s,
    );
  }

  duplicate(tab: Tab) {
    if (tab.kind === "local") this.openLocal(tab.shell ?? null);
    else if (tab.connect) {
      const s = this.sessions.find((x) => x.id === tab.sessionId);
      this.openSsh(tab.connect, tab.title, s);
    }
  }

  patch(key: string, p: TabPatch) {
    const t = this.tabs.find((x) => x.key === key);
    if (!t) return;
    Object.assign(t, p);
    if (p.hasSecret && t.sessionId) {
      const s = this.sessions.find((x) => x.id === t.sessionId);
      if (s) s.hasSecret = true;
    }
  }

  close(key: string) {
    const i = this.tabs.findIndex((t) => t.key === key);
    if (i < 0) return;
    // Terminal bileşeni kaldırılırken arka taraftaki oturumu da kapatır.
    this.tabs.splice(i, 1);
    if (this.activeKey === key) {
      this.activeKey = this.tabs[Math.min(i, this.tabs.length - 1)]?.key ?? null;
    }
  }

  cycle(dir: 1 | -1) {
    if (!this.tabs.length) return;
    const i = this.tabs.findIndex((t) => t.key === this.activeKey);
    this.activeKey = this.tabs[(i + dir + this.tabs.length) % this.tabs.length].key;
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
