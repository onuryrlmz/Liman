<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { openPath } from "@tauri-apps/plugin-opener";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import Icon from "./Icon.svelte";
  import { api, errText, type RdpInput } from "./api";
  import { openVia } from "./via";
  import { keyboardLayout, scancode, SC_ALT, SC_CTRL, SC_DELETE, SC_WIN } from "./rdp-keys";
  import { store, type Pane, type TabPatch } from "./tabs.svelte";

  let {
    tab,
    visible,
    focused,
    onUpdate,
    onFocus,
  }: { tab: Pane; visible: boolean; focused: boolean; onUpdate: (p: TabPatch) => void; onFocus: () => void } = $props();

  const isMac = navigator.platform.toLowerCase().includes("mac");

  let screen: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D | null = null;
  let rdpId: string | null = null;
  let viaId: string | null = null;
  let destroyed = false;

  let phase = $state<"password" | "connecting" | "open" | "closed" | "error">("connecting");
  let message = $state("");
  let username = $state("");
  let password = $state("");
  let remote = $state({ w: 0, h: 0 });
  let cursor = $state("default");

  // ---------- Bağlantı ----------

  /** Kabın boyutu (aygıt pikseli) ve ölçek; uzak masaüstü bu boyutta açılır. */
  function wanted() {
    const r = screen.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    return {
      width: Math.max(200, Math.round(r.width * dpr)) & ~1,
      height: Math.max(200, Math.round(r.height * dpr)),
      scale: Math.round(dpr * 100),
    };
  }

  async function start(pw: string | null) {
    cleanup();
    phase = "connecting";
    message = "";
    onUpdate({ status: "connecting" });
    const { host, port, jump } = tab.connect!;
    try {
      if (jump) viaId = await openVia(jump, () => {});
      if (destroyed) return cleanup();
      const size = wanted();
      rdpId = await api.rdpConnect({
        host,
        port,
        username: username.trim(),
        password: pw,
        sessionId: tab.sessionId ?? null,
        via: viaId,
        ...size,
        keyboardLayout: keyboardLayout(),
        shareDir: store.sessions.find((s) => s.id === tab.sessionId)?.rdpShare ?? null,
        onData,
      });
      if (destroyed) cleanup();
    } catch (e) {
      phase = "error";
      message = errText(e);
      onUpdate({ status: "error" });
    }
  }

  /** Kayıtlı parola yoksa önce sorar (NLA'lı sunucular parolasız bağlantıyı reddeder). */
  async function connect() {
    username = username || tab.connect?.username || "";
    const saved = tab.sessionId ? await api.sessionPassword(tab.sessionId).catch(() => null) : null;
    if (destroyed) return;
    if (saved || password) return start(password || null);
    phase = "password";
    onUpdate({ status: "connecting" });
  }

  function submitPassword(e: SubmitEvent) {
    e.preventDefault();
    const pw = password;
    password = "";
    start(pw);
  }

  // ---------- Sunucudan gelenler ----------

  function onData(d: Uint8Array) {
    const v = new DataView(d.buffer, d.byteOffset, d.byteLength);
    const u16 = (o: number) => v.getUint16(o, true);
    switch (d[0]) {
      case 1: {
        if (!ctx) return;
        const [x, y, w, h] = [u16(1), u16(3), u16(5), u16(7)];
        const px = new Uint8ClampedArray(d.buffer as ArrayBuffer, d.byteOffset + 9, w * h * 4);
        ctx.putImageData(new ImageData(px, w, h), x, y);
        break;
      }
      case 2:
        remote = { w: u16(1), h: u16(3) };
        canvas.width = remote.w;
        canvas.height = remote.h;
        ctx = canvas.getContext("2d", { alpha: false });
        break;
      case 3:
        setCursor(u16(1), u16(3), u16(5), u16(7), d.subarray(9));
        break;
      case 4:
        cursor = "default";
        break;
      case 5:
        cursor = "none";
        break;
      case 6: {
        const s = JSON.parse(new TextDecoder().decode(d.subarray(1))) as { state: string; message: string };
        if (s.state === "open") {
          phase = "open";
          onUpdate({ status: "open" });
          canvas?.focus();
          send({ t: "clipboardSync" });
        } else if (s.state === "error") {
          phase = "error";
          message = s.message;
          onUpdate({ status: "error" });
        } else if (s.state === "closed" && phase !== "error") {
          phase = "closed";
          onUpdate({ status: "closed" });
        }
        break;
      }
    }
  }

  function setCursor(hx: number, hy: number, w: number, h: number, rgba: Uint8Array) {
    if (!w || !h) return void (cursor = "none");
    const c = document.createElement("canvas");
    c.width = w;
    c.height = h;
    c.getContext("2d")!.putImageData(new ImageData(new Uint8ClampedArray(rgba.slice(0, w * h * 4)), w, h), 0, 0);
    // Tuval uzak piksel ölçeğinde gösteriliyor; imleci de aynı oranda küçült.
    cursor = `url(${c.toDataURL()}) ${hx} ${hy}, default`;
  }

  // ---------- Giriş ----------

  let queue: RdpInput[] = [];
  let flushing = false;
  function send(e: RdpInput) {
    if (!rdpId || phase !== "open") return;
    // Art arda fare hareketlerinde yalnızca sonuncusu gider.
    const last = queue[queue.length - 1];
    if (e.t === "move" && last?.t === "move") queue[queue.length - 1] = e;
    else queue.push(e);
    if (!flushing) {
      flushing = true;
      requestAnimationFrame(() => {
        const batch = queue;
        queue = [];
        flushing = false;
        if (rdpId && batch.length) api.rdpInput(rdpId, batch).catch(() => {});
      });
    }
  }

  function pos(e: MouseEvent) {
    const r = canvas.getBoundingClientRect();
    const x = Math.round(((e.clientX - r.left) / r.width) * remote.w);
    const y = Math.round(((e.clientY - r.top) / r.height) * remote.h);
    return { x: Math.min(Math.max(x, 0), remote.w - 1), y: Math.min(Math.max(y, 0), remote.h - 1) };
  }

  function onMove(e: PointerEvent) {
    send({ t: "move", ...pos(e) });
  }
  function onButton(e: PointerEvent, down: boolean) {
    e.preventDefault();
    if (down) {
      canvas.focus();
      canvas.setPointerCapture(e.pointerId);
    }
    send({ t: "move", ...pos(e) });
    send({ t: "button", b: e.button, down });
  }
  let wheelAcc = { x: 0, y: 0 };
  function onWheel(e: WheelEvent) {
    e.preventDefault();
    // Bir çentik 120 birim; dokunmatik yüzeylerin küçük adımları biriktirilir.
    const k = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? 800 : 1;
    wheelAcc.y += e.deltaY * k;
    wheelAcc.x += e.deltaX * k;
    for (const [axis, vertical] of [["y", true], ["x", false]] as const) {
      const steps = Math.trunc(wheelAcc[axis] / 40);
      if (!steps) continue;
      wheelAcc[axis] -= steps * 40;
      const delta = Math.max(-255, Math.min(255, -steps * 40));
      send({ t: "wheel", delta: vertical ? delta : -delta, vertical });
    }
  }

  function onKey(e: KeyboardEvent, down: boolean) {
    if (phase !== "open") return;
    // Uygulamanın kendi kısayolları (ör. Cmd+W, Ctrl+Tab) pencerede işlensin.
    if (e.defaultPrevented) return;
    e.preventDefault();
    e.stopPropagation();
    const sc = scancode(e.code, isMac);
    if (sc !== null) send({ t: "key", code: sc, down });
    else if (e.key.length === 1) send({ t: "unicode", ch: e.key, down });
  }

  function combo(codes: number[]) {
    for (const c of codes) send({ t: "key", code: c, down: true });
    for (const c of [...codes].reverse()) send({ t: "key", code: c, down: false });
    canvas.focus();
  }

  async function pasteAsTyping() {
    const t = await readText().catch(() => "");
    for (const ch of t.replace(/\r\n/g, "\n")) {
      if (ch === "\n") combo([0x1c]);
      else {
        send({ t: "unicode", ch, down: true });
        send({ t: "unicode", ch, down: false });
      }
    }
    canvas.focus();
  }

  // ---------- Boyut ----------

  let resizeTimer: ReturnType<typeof setTimeout> | undefined;
  let observer: ResizeObserver | undefined;
  function watchSize() {
    observer = new ResizeObserver(() => {
      if (phase !== "open" || !visible) return;
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => {
        const s = wanted();
        if (rdpId && (Math.abs(s.width - remote.w) > 8 || Math.abs(s.height - remote.h) > 8))
          api.rdpInput(rdpId, [{ t: "resize", ...s }]).catch(() => {});
      }, 400);
    });
    observer.observe(screen);
  }

  // ---------- Bilgisayardaki istemci (yedek) ----------

  let externalNote = $state("");
  async function openExternal() {
    externalNote = "";
    const { host, port, jump } = tab.connect!;
    try {
      const via = jump ? await openVia(jump, () => {}) : null;
      if (via) viaId = via;
      const r = await api.rdpPrepare(host, port, username || tab.connect?.username || "", via, tab.sessionId ?? null);
      if (r.hasPassword && !r.passwordGiven && tab.sessionId) {
        const secret = await api.sessionPassword(tab.sessionId).catch(() => null);
        if (secret) {
          await writeText(secret);
          externalNote = `Parola panoya kopyalandı; istemci sorunca ${isMac ? "⌘V" : "Ctrl+V"} ile yapıştırın.`;
        }
      }
      await openPath(r.file);
    } catch (e) {
      externalNote = errText(e);
    }
  }

  function cleanup() {
    if (rdpId) api.rdpClose(rdpId).catch(() => {});
    rdpId = null;
    ctx = null;
    if (viaId) api.termClose(viaId);
    viaId = null;
  }

  export function restart() {
    connect();
  }

  $effect(() => {
    if (visible && focused && phase === "open") canvas?.focus();
  });
  // Sekmeden çıkınca basılı kalan tuşlar uzak makinede takılı kalmasın.
  $effect(() => {
    if (!visible || !focused) send({ t: "releaseAll" });
  });

  onMount(() => {
    watchSize();
    connect();
  });
  onDestroy(() => {
    destroyed = true;
    observer?.disconnect();
    clearTimeout(resizeTimer);
    cleanup();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="rdp" onmousedown={onFocus} onfocusin={onFocus}>
  <div class="bar">
    <span class="title"><Icon name="monitor" size={13} /> {tab.connect?.host}:{tab.connect?.port}</span>
    {#if phase === "open"}<span class="size">{remote.w}×{remote.h}</span>{/if}
    <span class="spacer"></span>
    <button class="icon-btn" title="Ctrl+Alt+Del gönder" disabled={phase !== "open"} onclick={() => combo([SC_CTRL, SC_ALT, SC_DELETE])}>
      <Icon name="keyboard" size={14} />
    </button>
    <button class="icon-btn win" title="Windows tuşu" disabled={phase !== "open"} onclick={() => combo([SC_WIN])}>⊞</button>
    <button class="icon-btn" title="Panodaki metni yazarak gönder" disabled={phase !== "open"} onclick={pasteAsTyping}>
      <Icon name="copy" size={14} />
    </button>
    <button class="icon-btn" title="Bilgisayardaki RDP istemcisinde aç" onclick={openExternal}>
      <Icon name="external" size={14} />
    </button>
  </div>
  <div class="screen" bind:this={screen}>
    <canvas
      bind:this={canvas}
      tabindex="0"
      style:cursor
      style:aspect-ratio={remote.w && remote.h ? `${remote.w} / ${remote.h}` : undefined}
      class:hidden={phase !== "open"}
      onpointermove={onMove}
      onpointerdown={(e) => onButton(e, true)}
      onpointerup={(e) => onButton(e, false)}
      oncontextmenu={(e) => e.preventDefault()}
      onwheel={onWheel}
      onkeydown={(e) => onKey(e, true)}
      onkeyup={(e) => onKey(e, false)}
      onfocus={() => send({ t: "clipboardSync" })}
      onblur={() => send({ t: "releaseAll" })}
    ></canvas>
  </div>

  {#if phase !== "open"}
    <div class="overlay">
      {#if phase === "password"}
        <form class="card" onsubmit={submitPassword}>
          <Icon name="key" size={22} />
          <strong>Uzak masaüstü girişi</strong>
          <input bind:value={username} placeholder="Kullanıcı adı (ETKİALANI\kullanıcı)" spellcheck="false" />
          <!-- svelte-ignore a11y_autofocus -->
          <input type="password" bind:value={password} placeholder="Parola" autofocus />
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
          {#if phase !== "connecting"}
            <div class="actions">
              <button class="btn primary" onclick={connect}>Yeniden bağlan</button>
              <button class="btn" onclick={openExternal}>Bilgisayardaki istemcide aç</button>
            </div>
          {/if}
          {#if externalNote}<p>{externalNote}</p>{/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .rdp {
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
  .size {
    margin-left: 8px;
    font-size: 11px;
  }
  .spacer {
    flex: 1;
  }
  .icon-btn.win {
    font-size: 14px;
    line-height: 1;
  }
  .screen {
    flex: 1;
    min-height: 0;
    display: grid;
    place-items: center;
    overflow: hidden;
  }
  canvas {
    max-width: 100%;
    max-height: 100%;
    outline: none;
    image-rendering: auto;
  }
  canvas.hidden {
    visibility: hidden;
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
  .card input {
    width: 280px;
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
  .card small {
    color: var(--muted);
    font-size: 11.5px;
  }
  .actions {
    display: flex;
    gap: 8px;
  }
</style>
