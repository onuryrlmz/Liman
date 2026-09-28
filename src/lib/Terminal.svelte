<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import "@xterm/xterm/css/xterm.css";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { api, errText, AUTH_FAILED } from "./api";
  import type { Tab, TabPatch } from "./tabs.svelte";

  let {
    tab,
    active,
    onUpdate,
  }: { tab: Tab; active: boolean; onUpdate: (patch: TabPatch) => void } = $props();

  let el: HTMLDivElement;
  let term: Terminal;
  let fit: FitAddon;
  let termId: string | null = null;
  let closed = false;
  let unlisten: UnlistenFn | undefined;
  let ro: ResizeObserver | undefined;
  const isMac = navigator.platform.toLowerCase().includes("mac");

  // Terminal içinde basit satır okuyucu (kullanıcı adı / parola istemi için).
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

  async function start() {
    closed = false;
    onUpdate({ status: "connecting" });
    const { cols, rows } = term;
    try {
      if (tab.kind === "local") {
        termId = await api.localSpawn(tab.shell ?? null, cols, rows, write);
      } else {
        const req = { ...tab.connect! };
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
          info(`${req.username}@${req.host}:${req.port} adresine bağlanılıyor...`);
          try {
            termId = await api.sshConnect(req, cols, rows, write);
            break;
          } catch (e) {
            const msg = errText(e);
            if (!msg.includes(AUTH_FAILED) || attempt >= 2) throw e;
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
      }
      onUpdate({ status: "open", termId });
      if (fitNow()) api.termResize(termId!, term.cols, term.rows);
    } catch (e) {
      const msg = errText(e);
      fail(msg.includes(AUTH_FAILED) ? "Kimlik doğrulama başarısız." : msg);
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
    term.reset();
    start();
  }

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

  onMount(async () => {
    term = new Terminal({
      fontFamily: '"JetBrains Mono", "Cascadia Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace',
      fontSize: 13,
      lineHeight: 1.15,
      cursorBlink: true,
      scrollback: 10000,
      allowProposedApi: true,
      macOptionIsMeta: true,
      theme: {
        background: "#11151c",
        foreground: "#d6dde6",
        cursor: "#7fd1b9",
        selectionBackground: "#2d4f67",
        black: "#1c2129",
        red: "#e06c75",
        green: "#98c379",
        yellow: "#e5c07b",
        blue: "#61afef",
        magenta: "#c678dd",
        cyan: "#56b6c2",
        white: "#d6dde6",
        brightBlack: "#5c6370",
        brightRed: "#ff7a85",
        brightGreen: "#b5e890",
        brightYellow: "#ffd68a",
        brightBlue: "#7cc4ff",
        brightMagenta: "#de9bf0",
        brightCyan: "#7fd1dc",
        brightWhite: "#ffffff",
      },
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new WebLinksAddon((_e, uri) => openUrl(uri)));
    term.open(el);
    fitNow();

    term.onData((d) => {
      if (reader) return feedReader(d);
      if (termId) return void api.termWrite(termId, d);
      if (closed && (d === "r" || d === "R")) restart();
    });
    // MobaXterm gibi: seçileni otomatik kopyala.
    term.onSelectionChange(() => {
      const sel = term.getSelection();
      if (sel) writeText(sel).catch(() => {});
    });
    // Linux/Windows'ta Ctrl+Shift+C / Ctrl+Shift+V.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown" || isMac) return true;
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
        term.focus();
      });
    }
  });

  onDestroy(() => {
    ro?.disconnect();
    unlisten?.();
    if (termId) api.termClose(termId);
    term?.dispose();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="term"
  bind:this={el}
  oncontextmenu={(e) => {
    e.preventDefault();
    paste();
  }}
></div>

<style>
  .term {
    position: absolute;
    inset: 0;
    padding: 6px 0 0 8px;
    background: #11151c;
  }
  .term :global(.xterm) {
    height: 100%;
  }
  .term :global(.xterm-viewport) {
    background: #11151c !important;
  }
</style>
