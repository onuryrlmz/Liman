<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { readText } from "@tauri-apps/plugin-clipboard-manager";
  import RFB from "@novnc/novnc";
  import Icon from "./Icon.svelte";
  import { api, errText } from "./api";
  import { openVia } from "./via";
  import type { Pane, TabPatch } from "./tabs.svelte";

  let {
    tab,
    visible,
    focused,
    onUpdate,
    onFocus,
  }: { tab: Pane; visible: boolean; focused: boolean; onUpdate: (p: TabPatch) => void; onFocus: () => void } = $props();

  let screen: HTMLDivElement;
  let rfb: RFB | null = null;
  let streamId: string | null = null;
  let viaId: string | null = null;
  let bridge: Bridge | null = null;
  let unlisten: UnlistenFn | undefined;

  let phase = $state<"connecting" | "password" | "open" | "closed" | "error">("connecting");
  let message = $state("");
  let log = $state<string[]>([]);
  let password = $state("");
  let username = $state("");
  let needUser = $state(false);
  let scale = $state(true);
  let viewOnly = $state(false);

  /** noVNC'nin WebSocket yerine kullandığı, Tauri üzerinden giden kanal. */
  class Bridge {
    binaryType = "arraybuffer";
    protocol = "";
    readyState = "open";
    onopen: ((e: unknown) => void) | null = null;
    onmessage: ((e: { data: ArrayBuffer }) => void) | null = null;
    onclose: ((e: { code: number; reason: string; wasClean: boolean }) => void) | null = null;
    onerror: ((e: unknown) => void) | null = null;
    private id: string;
    constructor(id: string) {
      this.id = id;
    }
    send(data: Uint8Array) {
      // noVNC gönderim tamponunu yeniden kullanır; kopyalamadan göndermeyelim.
      api.streamWrite(this.id, new Uint8Array(data)).catch(() => {});
    }
    close() {
      if (this.readyState === "closed") return;
      this.readyState = "closed";
      api.streamClose(this.id);
      this.onclose?.({ code: 1000, reason: "", wasClean: true });
    }
  }

  function addLog(s: string) {
    if (s) log = [...log.slice(-8), s];
  }

  let destroyed = false;
  /** Akış kimliği arayüze ulaşmadan kapanan akışlar (sunucu hemen bağlantıyı kesti). */
  const exited = new Set<string>();

  function markClosed() {
    if (bridge && bridge.readyState !== "closed") {
      bridge.readyState = "closed";
      bridge.onclose?.({ code: 1006, reason: "", wasClean: false });
    }
  }

  async function connect() {
    cleanup();
    phase = "connecting";
    message = "";
    log = [];
    onUpdate({ status: "connecting" });
    const { host, port, jump } = tab.connect!;
    try {
      // Dinleyici akış açılmadan kurulur ki erken kapanış kaçmasın.
      unlisten = await listen<{ id: string }>("term-exit", (ev) => {
        exited.add(ev.payload.id);
        if (ev.payload.id === streamId) markClosed();
      });
      if (jump) viaId = await openVia(jump, addLog);
      if (destroyed) return cleanup();
      addLog(`VNC: ${host}:${port}${viaId ? " (SSH tüneli üzerinden)" : ""}`);
      const pending: Uint8Array[] = [];
      streamId = await api.streamOpen(host, port, viaId, (d) => {
        if (bridge?.onmessage) bridge.onmessage({ data: d.slice().buffer });
        else pending.push(d.slice());
      });
      if (destroyed) return cleanup();
      bridge = new Bridge(streamId);
      const saved = tab.sessionId ? await api.sessionPassword(tab.sessionId).catch(() => null) : null;
      if (destroyed) return cleanup();
      const user = tab.connect?.username || undefined;
      rfb = new RFB(screen, bridge as unknown as WebSocket, {
        credentials: saved || user ? { username: user, password: saved ?? undefined } : undefined,
      });
      // Kanal hazırlanırken gelen ilk baytlar (sunucu sürümü) kaybolmasın.
      for (const d of pending) bridge.onmessage?.({ data: d.buffer as ArrayBuffer });
      if (exited.has(streamId)) markClosed();
      rfb.scaleViewport = scale;
      rfb.resizeSession = false;
      rfb.viewOnly = viewOnly;
      rfb.addEventListener("connect", () => {
        phase = "open";
        onUpdate({ status: "open" });
        rfb?.focus();
      });
      rfb.addEventListener("disconnect", (e) => {
        const clean = (e as CustomEvent).detail?.clean;
        if (phase !== "error") {
          phase = "closed";
          message = clean ? "Bağlantı kapandı." : "Bağlantı beklenmedik şekilde koptu.";
        }
        onUpdate({ status: phase === "error" ? "error" : "closed" });
      });
      rfb.addEventListener("credentialsrequired", (e) => {
        // macOS Ekran Paylaşımı gibi sunucular kullanıcı adı da ister.
        const types: string[] = (e as CustomEvent).detail?.types ?? ["password"];
        needUser = types.includes("username");
        username = username || tab.connect?.username || "";
        phase = "password";
        onUpdate({ status: "connecting" });
      });
      rfb.addEventListener("securityfailure", (e) => {
        phase = "error";
        message = `Güvenlik doğrulaması başarısız: ${(e as CustomEvent).detail?.reason ?? "parola yanlış olabilir"}`;
      });
      rfb.addEventListener("desktopname", (e) => onUpdate({ remoteTitle: (e as CustomEvent).detail?.name }));
    } catch (e) {
      if (destroyed) return cleanup();
      phase = "error";
      message = errText(e);
      onUpdate({ status: "error" });
    }
  }

  function sendPassword(e: Event) {
    e.preventDefault();
    phase = "connecting";
    rfb?.sendCredentials(needUser ? { username, password } : { password });
    password = "";
  }

  async function pasteClipboard() {
    const t = await readText().catch(() => "");
    if (t) rfb?.clipboardPasteFrom(t);
  }

  function cleanup() {
    unlisten?.();
    unlisten = undefined;
    try {
      rfb?.disconnect();
    } catch {}
    rfb = null;
    bridge?.close();
    bridge = null;
    if (streamId) api.streamClose(streamId);
    streamId = null;
    if (viaId) api.termClose(viaId);
    viaId = null;
  }

  export function restart() {
    connect();
  }

  $effect(() => {
    if (rfb) rfb.scaleViewport = scale;
  });
  $effect(() => {
    if (rfb) rfb.viewOnly = viewOnly;
  });
  $effect(() => {
    if (visible && focused && phase === "open") rfb?.focus();
  });

  onMount(connect);
  onDestroy(() => {
    destroyed = true;
    cleanup();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="vnc" onmousedown={onFocus} onfocusin={onFocus}>
  <div class="bar">
    <span class="title"><Icon name="monitor" size={13} /> {tab.connect?.host}:{tab.connect?.port}</span>
    <span class="spacer"></span>
    <button class="icon-btn" title="Ctrl+Alt+Del gönder" disabled={phase !== "open"} onclick={() => rfb?.sendCtrlAltDel()}>
      <Icon name="keyboard" size={14} />
    </button>
    <button class="icon-btn" title="Panodaki metni uzak makineye gönder" disabled={phase !== "open"} onclick={pasteClipboard}>
      <Icon name="copy" size={14} />
    </button>
    <button class="icon-btn" class:on={scale} title={scale ? "Gerçek boyut" : "Pencereye sığdır"} onclick={() => (scale = !scale)}>
      <Icon name="expand" size={14} />
    </button>
    <label class="vo"><input type="checkbox" bind:checked={viewOnly} /> Yalnızca izle</label>
  </div>
  <div class="screen" bind:this={screen}></div>

  {#if phase !== "open"}
    <div class="overlay">
      {#if phase === "password"}
        <form class="card" onsubmit={sendPassword}>
          <Icon name="key" size={22} />
          <strong>{needUser ? "VNC girişi" : "VNC parolası"}</strong>
          {#if needUser}
            <!-- svelte-ignore a11y_autofocus -->
            <input bind:value={username} placeholder="Kullanıcı adı" autofocus spellcheck="false" />
            <input type="password" bind:value={password} placeholder="Parola" />
          {:else}
            <!-- svelte-ignore a11y_autofocus -->
            <input type="password" bind:value={password} autofocus />
          {/if}
          <button class="btn primary" type="submit">Bağlan</button>
          <small>Oturum ayarlarından parolayı kaydedebilirsiniz.</small>
        </form>
      {:else}
        <div class="card">
          <Icon name="monitor" size={26} />
          <strong>
            {phase === "connecting" ? "Bağlanılıyor…" : phase === "error" ? "Bağlanılamadı" : "Bağlantı kapandı"}
          </strong>
          {#if message}<p class:err={phase === "error"}>{message}</p>{/if}
          {#if log.length}<pre>{log.join("\n")}</pre>{/if}
          {#if phase !== "connecting"}<button class="btn primary" onclick={connect}>Yeniden bağlan</button>{/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .vnc {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: #0a0d12;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    color: var(--muted);
  }
  .title {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .spacer {
    flex: 1;
  }
  .icon-btn.on {
    color: var(--accent);
  }
  .vo {
    display: flex;
    align-items: center;
    gap: 5px;
    margin-left: 6px;
  }
  .screen {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .overlay {
    position: absolute;
    inset: 33px 0 0;
    display: grid;
    place-items: center;
    background: rgb(10 13 18 / 0.85);
  }
  .card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    max-width: 440px;
    padding: 24px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--panel);
    color: var(--text);
    text-align: center;
  }
  .card :global(svg) {
    color: var(--accent);
  }
  .card p {
    margin: 0;
    font-size: 13px;
    color: var(--muted);
  }
  .card p.err {
    color: var(--danger);
  }
  .card pre {
    margin: 0;
    max-width: 100%;
    overflow: auto;
    font-size: 11px;
    color: var(--muted);
    text-align: left;
    white-space: pre-wrap;
  }
  .card small {
    color: var(--muted);
    font-size: 11.5px;
  }
</style>
