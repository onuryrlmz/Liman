<script lang="ts">
  import { onMount, onDestroy, tick } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import { SearchAddon } from "@xterm/addon-search";
  import "@xterm/xterm/css/xterm.css";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import Icon from "./Icon.svelte";
  import { api, errText, AUTH_FAILED, HOST_KEY_REJECTED, type ConnectRequest } from "./api";
  import { settings } from "./settings.svelte";
  import type { Tab, TabPatch } from "./tabs.svelte";

  let {
    tab,
    active,
    onUpdate,
  }: { tab: Tab; active: boolean; onUpdate: (patch: TabPatch) => void } = $props();

  let el: HTMLDivElement;
  let term: Terminal;
  let fit: FitAddon;
  let search: SearchAddon;
  let termId: string | null = null;
  let closed = false;
  let unlisten: UnlistenFn | undefined;
  let ro: ResizeObserver | undefined;
  const isMac = navigator.platform.toLowerCase().includes("mac");

  /** Son başarılı bağlantının bilgileri (yazılan parola dahil); yeniden bağlanmada kullanılır. */
  let lastReq: ConnectRequest | null = null;

  // ---------- Terminal içinde satır okuyucu (kullanıcı adı / parola) ----------
  type LineReader = { buf: string; hidden: boolean; resolve: (v: string | null) => void };
  let reader: LineReader | null = null;

  function readLine(prompt: string, hidden = false): Promise<string | null> {
    term.write(prompt);
    return new Promise((resolve) => (reader = { buf: "", hidden, resolve }));
  }

  function feedReader(data: string) {
    if (!reader) return;
    for (const ch of data) {
      const r: LineReader | null = reader;
      if (!r) return;
      if (ch === "\r" || ch === "\n") {
        term.write("\r\n");
        reader = null;
        r.resolve(r.buf);
      } else if (ch === "\x03" || ch === "\x1b") {
        term.write("^C\r\n");
        reader = null;
        r.resolve(null);
      } else if (ch === "\x7f" || ch === "\b") {
        if (r.buf.length) {
          r.buf = r.buf.slice(0, -1);
          if (!r.hidden) term.write("\b \b");
        }
      } else if (ch >= " ") {
        r.buf += ch;
        if (!r.hidden) term.write(ch);
      }
    }
  }

  const write = (d: Uint8Array) => term.write(d);
  const info = (s: string) => term.write(`\x1b[90m${s}\x1b[0m\r\n`);
  const error = (s: string) => term.write(`\x1b[31m${s}\x1b[0m\r\n`);

  // ---------- Bağlanma ----------

  async function start() {
    closed = false;
    cancelReconnect();
    onUpdate({ status: "connecting" });
    const { cols, rows } = term;
    try {
      if (tab.kind === "local") {
        termId = await api.localSpawn(tab.shell ?? null, cols, rows, write);
      } else {
        const req: ConnectRequest = { ...tab.connect!, secret: lastReq?.secret ?? tab.connect!.secret };
        if (!req.username) {
          const u = await readLine("login as: ");
          if (!u) return fail("İptal edildi");
          req.username = u;
          onUpdate({ connect: { ...tab.connect!, username: u } });
        }
        const hasSaved = !!tab.sessionId && tab.hasSecret;
        if (req.auth === "password" && !hasSaved && !req.secret) {
          const pw = await readLine(`${req.username}@${req.host} parolası: `, true);
          if (pw === null) return fail("İptal edildi");
          req.secret = pw;
        }
        for (let attempt = 0; ; attempt++) {
          const via = req.jump ? " (atlama sunucusu üzerinden)" : "";
          info(`${req.sftpOnly ? "SFTP: " : ""}${req.username}@${req.host}:${req.port} adresine bağlanılıyor${via}...`);
          try {
            termId = await api.sshConnect(req, cols, rows, write);
            break;
          } catch (e) {
            const msg = errText(e);
            if (!msg.includes(AUTH_FAILED) || msg.includes("Atlama") || attempt >= 2) throw e;
            error("Kimlik doğrulama başarısız.");
            const pw = await readLine(`${req.username}@${req.host} parolası: `, true);
            if (pw === null) return fail("İptal edildi");
            req.secret = pw;
            if (tab.sessionId) {
              const ans = await readLine("Parola sistem kasasına kaydedilsin mi? [e/H] ");
              req.saveSecret = ans?.trim().toLowerCase().startsWith("e") ?? false;
              if (req.saveSecret) onUpdate({ hasSecret: true });
            }
          }
        }
        lastReq = { ...req, saveSecret: false };
      }
      reconnectAttempt = 0;
      onUpdate({ status: "open", termId });
      if (fitNow()) api.termResize(termId!, term.cols, term.rows);
    } catch (e) {
      const msg = errText(e);
      if (reconnectAttempt > 0 && !msg.includes(AUTH_FAILED) && !msg.includes(HOST_KEY_REJECTED)) {
        error(msg);
        return scheduleReconnect();
      }
      reconnectAttempt = 0;
      fail(
        msg.includes(AUTH_FAILED) && !msg.includes("Atlama")
          ? "Kimlik doğrulama başarısız."
          : msg.includes(HOST_KEY_REJECTED)
            ? "Sunucu anahtarı onaylanmadı, bağlantı kurulmadı."
            : msg,
      );
    }
  }

  function fail(msg: string) {
    error(msg);
    closed = true;
    onUpdate({ status: "error", termId: null });
    info("Yeniden bağlanmak için R tuşuna basın.");
  }

  export function restart() {
    if (termId) return;
    reconnectAttempt = 0;
    term.reset();
    start();
  }

  // ---------- Otomatik yeniden bağlanma ----------
  // Bağlantı çıkış kodu olmadan koparsa (ağ değişti, bilgisayar uyudu…) artan aralıklarla dener.

  const MAX_RECONNECT = 8;
  let reconnectAttempt = 0;
  let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  let countdownTimer: ReturnType<typeof setInterval> | undefined;

  function cancelReconnect() {
    clearTimeout(reconnectTimer);
    clearInterval(countdownTimer);
    reconnectTimer = countdownTimer = undefined;
  }

  function scheduleReconnect() {
    if (reconnectAttempt >= MAX_RECONNECT) {
      reconnectAttempt = 0;
      return fail(`${MAX_RECONNECT} denemede bağlanılamadı.`);
    }
    reconnectAttempt++;
    let left = Math.min(2 ** reconnectAttempt, 30);
    closed = true;
    onUpdate({ status: "connecting", termId: null });
    const line = () =>
      `\r\x1b[2K\x1b[33mBağlantı koptu. ${left} sn içinde yeniden bağlanılacak (deneme ${reconnectAttempt}/${MAX_RECONNECT}) — iptal: Esc, şimdi: R\x1b[0m`;
    term.write(line());
    countdownTimer = setInterval(() => {
      left--;
      if (left > 0) term.write(line());
    }, 1000);
    reconnectTimer = setTimeout(() => {
      cancelReconnect();
      term.write("\r\n");
      start();
    }, left * 1000);
  }

  // ---------- Boyut ----------

  function fitNow() {
    if (!el || el.offsetWidth === 0 || el.offsetHeight === 0) return false;
    fit.fit();
    return true;
  }

  let resizeTimer: ReturnType<typeof setTimeout>;
  function scheduleResize() {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      if (fitNow() && termId) api.termResize(termId, term.cols, term.rows);
    }, 60);
  }

  async function paste() {
    const text = await readText().catch(() => "");
    if (text) term.paste(text);
  }

  // ---------- Arama ----------

  let searchOpen = $state(false);
  let query = $state("");
  let caseSensitive = $state(false);
  let regex = $state(false);
  let results = $state<{ index: number; count: number } | null>(null);
  let searchInput = $state<HTMLInputElement>();

  const searchOpts = () => ({
    caseSensitive,
    regex,
    decorations: {
      matchBackground: "#5c4a1a",
      matchOverviewRuler: "#e5c07b",
      activeMatchBackground: "#b58900",
      activeMatchColorOverviewRuler: "#ffd68a",
    },
  });

  export async function openSearch() {
    searchOpen = true;
    const sel = term.getSelection();
    if (sel && !sel.includes("\n")) query = sel;
    await tick();
    searchInput?.select();
  }

  function closeSearch() {
    searchOpen = false;
    search.clearDecorations();
    results = null;
    term.focus();
  }

  function find(backwards = false) {
    if (!query) {
      search.clearDecorations();
      results = null;
      return;
    }
    if (backwards) search.findPrevious(query, searchOpts());
    else search.findNext(query, searchOpts());
  }

  // ---------- Ayarlar ----------

  function applySettings() {
    const s = settings.value;
    term.options.fontSize = s.fontSize;
    term.options.fontFamily = s.fontFamily;
    term.options.lineHeight = s.lineHeight;
    term.options.cursorStyle = s.cursorStyle;
    term.options.cursorBlink = s.cursorBlink;
    term.options.scrollback = s.scrollback;
    term.options.theme = settings.termTheme;
    scheduleResize();
  }

  $effect(() => {
    // Değişiklikleri izle.
    JSON.stringify(settings.value);
    if (term) applySettings();
  });

  const bg = $derived(settings.termTheme.background ?? "#11151c");

  onMount(async () => {
    term = new Terminal({
      allowProposedApi: true,
      macOptionIsMeta: true,
    });
    fit = new FitAddon();
    search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.loadAddon(new WebLinksAddon((_e, uri) => openUrl(uri)));
    applySettings();
    term.open(el);
    fitNow();

    search.onDidChangeResults((r) => (results = r.resultCount ? { index: r.resultIndex, count: r.resultCount } : { index: -1, count: 0 }));

    term.onData((d) => {
      if (reader) return feedReader(d);
      if (termId) return void api.termWrite(termId, d);
      if (reconnectTimer) {
        if (d === "\x1b" || d === "\x03") {
          cancelReconnect();
          reconnectAttempt = 0;
          term.write("\r\n");
          return fail("Yeniden bağlanma iptal edildi.");
        }
        if (d === "r" || d === "R") {
          cancelReconnect();
          term.write("\r\n");
          return void start();
        }
        return;
      }
      if (closed && (d === "r" || d === "R")) restart();
    });
    term.onSelectionChange(() => {
      if (!settings.value.copyOnSelect) return;
      const sel = term.getSelection();
      if (sel) writeText(sel).catch(() => {});
    });
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      const mod = isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && e.shiftKey;
      if (mod && e.code === "KeyF") {
        e.preventDefault();
        openSearch();
        return false;
      }
      if (isMac) return true;
      // Linux/Windows'ta Ctrl+Shift+C / Ctrl+Shift+V.
      if (e.ctrlKey && e.shiftKey && e.code === "KeyC") {
        const sel = term.getSelection();
        if (sel) writeText(sel);
        return false;
      }
      if (e.ctrlKey && e.shiftKey && e.code === "KeyV") {
        paste();
        return false;
      }
      return true;
    });
    term.onTitleChange((t) => onUpdate({ remoteTitle: t }));

    ro = new ResizeObserver(scheduleResize);
    ro.observe(el);

    unlisten = await listen<{ id: string; code: number | null }>("term-exit", (ev) => {
      if (!termId || ev.payload.id !== termId) return;
      const id = termId;
      termId = null;
      closed = true;
      api.termClose(id);
      const code = ev.payload.code;
      // Çıkış kodu yoksa kabuk kapanmamış, bağlantı kopmuştur.
      if (tab.kind !== "local" && code == null && settings.value.autoReconnect) {
        term.write("\r\n");
        return scheduleReconnect();
      }
      term.write(
        `\r\n\x1b[90m[Oturum sonlandı${code != null ? `, çıkış kodu ${code}` : ""}. Yeniden başlatmak için R]\x1b[0m\r\n`,
      );
      onUpdate({ status: "closed", termId: null });
    });

    start();
  });

  $effect(() => {
    if (active && term) {
      requestAnimationFrame(() => {
        scheduleResize();
        if (!searchOpen) term.focus();
      });
    }
  });

  onDestroy(() => {
    cancelReconnect();
    ro?.disconnect();
    unlisten?.();
    if (termId) api.termClose(termId);
    term?.dispose();
  });
</script>

<div class="wrap" style:background={bg}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="term"
    bind:this={el}
    oncontextmenu={(e) => {
      if (!settings.value.rightClickPaste) return;
      e.preventDefault();
      paste();
    }}
  ></div>

  {#if searchOpen}
    <div class="search" role="search">
      <Icon name="search" size={13} />
      <input
        bind:this={searchInput}
        bind:value={query}
        placeholder="Terminalde ara"
        spellcheck="false"
        oninput={() => find()}
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            find(e.shiftKey);
          } else if (e.key === "Escape") {
            e.preventDefault();
            closeSearch();
          }
        }}
      />
      <span class="count">
        {#if results}{results.count ? `${results.index + 1}/${results.count}` : "Yok"}{/if}
      </span>
      <button class="opt" class:on={caseSensitive} title="Büyük/küçük harf duyarlı" onclick={() => ((caseSensitive = !caseSensitive), find())}>Aa</button>
      <button class="opt" class:on={regex} title="Düzenli ifade" onclick={() => ((regex = !regex), find())}>.*</button>
      <button class="icon-btn" title="Önceki (Shift+Enter)" onclick={() => find(true)}><Icon name="up" size={13} /></button>
      <button class="icon-btn down" title="Sonraki (Enter)" onclick={() => find()}><Icon name="up" size={13} /></button>
      <button class="icon-btn" title="Kapat (Esc)" onclick={closeSearch}><Icon name="x" size={13} /></button>
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: absolute;
    inset: 0;
  }
  .term {
    position: absolute;
    inset: 0;
    padding: 6px 0 0 8px;
  }
  .term :global(.xterm) {
    height: 100%;
  }
  .term :global(.xterm-viewport) {
    background: transparent !important;
  }
  .search {
    position: absolute;
    top: 8px;
    right: 18px;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 4px 4px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--panel-2);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.4);
    color: var(--muted);
  }
  .search input {
    width: 200px;
    padding: 4px 6px;
    border: 0;
    background: none;
  }
  .count {
    min-width: 44px;
    font-size: 11.5px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .opt {
    padding: 2px 6px;
    border: 1px solid transparent;
    border-radius: 4px;
    background: none;
    color: var(--muted);
    font: 600 11px "JetBrains Mono", Menlo, monospace;
    cursor: pointer;
  }
  .opt.on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .down :global(svg) {
    transform: rotate(180deg);
  }
</style>
