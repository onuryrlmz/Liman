<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import Terminal from "$lib/Terminal.svelte";
  import SessionList from "$lib/SessionList.svelte";
  import SftpPanel from "$lib/SftpPanel.svelte";
  import SessionDialog from "$lib/SessionDialog.svelte";
  import TunnelDialog from "$lib/TunnelDialog.svelte";
  import AboutDialog from "$lib/AboutDialog.svelte";
  import ContextMenu, { type MenuItem } from "$lib/ContextMenu.svelte";
  import Icon from "$lib/Icon.svelte";
  import { api, formatSize, parseQuick, type Session, type Transfer } from "$lib/api";
  import { store, type Tab } from "$lib/tabs.svelte";

  const isMac = navigator.platform.toLowerCase().includes("mac");
  const mod = isMac ? "⌘" : "Ctrl+Shift+";

  let sidebarTab = $state<"sessions" | "sftp">("sessions");
  let sidebarOpen = $state(true);
  let sidebarWidth = $state(loadWidth());
  let editing = $state<{ session: Session | null; folder?: string | null } | null>(null);
  let tunnelTab = $state<Tab | null>(null);
  let aboutOpen = $state(false);
  let menu = $state<{ x: number; y: number; items: (MenuItem | null)[] } | null>(null);
  let quick = $state("");
  let terminals: Record<string, Terminal> = {};

  const active = $derived(store.active);
  const sftpTermId = $derived(active?.kind === "ssh" && active.status === "open" ? active.termId : null);

  function loadWidth() {
    try {
      return Number(localStorage.getItem("sidebarWidth")) || 270;
    } catch {
      return 270;
    }
  }

  // MobaXterm gibi: SSH bağlanınca SFTP paneline geç.
  let lastAuto: string | null = null;
  $effect(() => {
    if (sftpTermId && sftpTermId !== lastAuto) {
      lastAuto = sftpTermId;
      sidebarTab = "sftp";
      sidebarOpen = true;
    }
  });

  onMount(() => {
    store.loadSessions();
    api.localShells().then((s) => {
      store.shells = s;
    });
    const un = listen<Transfer>("transfer", (e) => store.upsertTransfer(e.payload));
    return () => {
      un.then((f) => f());
    };
  });

  function quickConnect(e: Event) {
    e.preventDefault();
    const q = parseQuick(quick);
    if (!q) {
      store.notify("Biçim: kullanıcı@sunucu:port", "error");
      return;
    }
    store.openSsh(
      { host: q.host, port: q.port, username: q.username, auth: "auto", sftpOnly: q.sftpOnly },
      quick.trim().replace(/^ssh:\/\//i, ""),
    );
    quick = "";
  }

  function shellMenu(ev: MouseEvent) {
    const r = (ev.currentTarget as HTMLElement).getBoundingClientRect();
    menu = {
      x: r.left,
      y: r.bottom + 4,
      items: store.shells.length
        ? store.shells.map((s) => ({ label: s, icon: "terminal", action: () => store.openLocal(s) }))
        : [{ label: "Varsayılan kabuk", icon: "terminal", action: () => store.openLocal() }],
    };
  }

  function tabMenu(ev: MouseEvent, tab: Tab) {
    ev.preventDefault();
    menu = {
      x: ev.clientX,
      y: ev.clientY,
      items: [
        { label: "Yeniden bağlan", icon: "refresh", disabled: tab.status === "open" || tab.status === "connecting", action: () => terminals[tab.key]?.restart() },
        { label: "Çoğalt", icon: "copy", action: () => store.duplicate(tab) },
        tab.kind !== "local" ? { label: "Tüneller…", icon: "tunnel", disabled: tab.status !== "open", action: () => (tunnelTab = tab) } : null,
        null,
        { label: "Kapat", icon: "x", action: () => store.close(tab.key) },
        { label: "Diğerlerini kapat", action: () => store.tabs.filter((t) => t.key !== tab.key).forEach((t) => store.close(t.key)) },
      ].filter((x, i) => x !== null || i === 3),
    };
  }

  function onKey(e: KeyboardEvent) {
    const primary = isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && e.shiftKey;
    if (e.ctrlKey && e.key === "Tab") {
      store.cycle(e.shiftKey ? -1 : 1);
    } else if (primary && e.code === "KeyT") {
      store.openLocal();
    } else if (primary && e.code === "KeyW") {
      if (store.activeKey) store.close(store.activeKey);
    } else if (primary && e.code === "KeyN") {
      editing = { session: null };
    } else if (primary && e.code === "KeyB") {
      sidebarOpen = !sidebarOpen;
    } else if (isMac && e.metaKey && /^Digit[1-9]$/.test(e.code)) {
      const t = store.tabs[Number(e.code.slice(5)) - 1];
      if (t) store.activeKey = t.key;
    } else return;
    e.preventDefault();
    e.stopPropagation();
  }

  function startResize(e: PointerEvent) {
    const startX = e.clientX;
    const start = sidebarWidth;
    const move = (ev: PointerEvent) => (sidebarWidth = Math.max(200, Math.min(600, start + ev.clientX - startX)));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      try {
        localStorage.setItem("sidebarWidth", String(sidebarWidth));
      } catch {}
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function describe(t: Tab | null) {
    if (!t) return "Hazır";
    const st = { connecting: "bağlanıyor…", open: "bağlı", closed: "kapandı", error: "hata" }[t.status];
    if (t.kind === "local") return `Yerel • ${t.shell ?? "varsayılan kabuk"} • ${st}`;
    const c = t.connect!;
    return `${t.kind === "sftp" ? "SFTP" : "SSH"} • ${c.username ? c.username + "@" : ""}${c.host}:${c.port} • ${st}`;
  }
</script>

<svelte:window onkeydowncapture={onKey} />

<div class="app">
  <header class="toolbar">
    <button class="brand" onclick={() => (aboutOpen = true)} title="Liman hakkında">
      <span class="logo"><Icon name="anchor" size={15} /></span> Liman
    </button>
    <button class="tool" onclick={() => (editing = { session: null })} title="Yeni SSH oturumu ({mod}N)">
      <Icon name="server" size={18} /><span>Oturum</span>
    </button>
    <div class="split">
      <button class="tool" onclick={() => store.openLocal()} title="Yerel terminal ({mod}T)">
        <Icon name="terminal" size={18} /><span>Terminal</span>
      </button>
      <button class="tool caret" onclick={shellMenu} title="Kabuk seç"><Icon name="chevronDown" size={13} /></button>
    </div>
    <button
      class="tool"
      disabled={!active || active.kind === "local" || active.status !== "open"}
      onclick={() => (tunnelTab = active)}
      title="Aktif SSH oturumu için port yönlendirme"
    >
      <Icon name="tunnel" size={18} /><span>Tüneller</span>
    </button>
    <button class="tool" class:on={sidebarOpen} onclick={() => (sidebarOpen = !sidebarOpen)} title="Kenar çubuğu ({mod}B)">
      <Icon name="sidebar" size={18} /><span>Panel</span>
    </button>
    <form class="quick" onsubmit={quickConnect}>
      <Icon name="bolt" size={14} />
      <input bind:value={quick} placeholder="Hızlı bağlan: kullanıcı@sunucu:port  (sftp://… yalnızca SFTP)" spellcheck="false" autocapitalize="off" />
    </form>
    <button class="icon-btn about-btn" onclick={() => (aboutOpen = true)} title="Hakkında" aria-label="Hakkında">
      <Icon name="info" size={17} />
    </button>
  </header>

  <div class="main">
    {#if sidebarOpen}
      <aside class="sidebar" style:width="{sidebarWidth}px">
        <div class="side-tabs">
          <button class:on={sidebarTab === "sessions"} onclick={() => (sidebarTab = "sessions")}>
            <Icon name="server" size={14} /> Oturumlar
          </button>
          <button class:on={sidebarTab === "sftp"} onclick={() => (sidebarTab = "sftp")}>
            <Icon name="folder" size={14} /> SFTP
          </button>
        </div>
        <div class="side-body">
          {#if sidebarTab === "sessions"}
            <SessionList onEdit={(s, folder) => (editing = { session: s, folder })} />
          {:else if sftpTermId}
            {#key sftpTermId}
              <SftpPanel termId={sftpTermId} />
            {/key}
          {:else}
            <div class="placeholder">
              <Icon name="folder" size={28} />
              <p>SFTP tarayıcısı, bağlı bir SSH sekmesi seçildiğinde burada açılır.</p>
            </div>
          {/if}
        </div>
      </aside>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="resizer" onpointerdown={startResize}></div>
    {/if}

    <section class="work">
      <nav class="tabbar">
        {#each store.tabs as tab (tab.key)}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div
            class="tab"
            class:on={tab.key === store.activeKey}
            onclick={() => (store.activeKey = tab.key)}
            onauxclick={(e) => e.button === 1 && store.close(tab.key)}
            oncontextmenu={(e) => tabMenu(e, tab)}
            title={describe(tab)}
          >
            <span class="st {tab.status}"></span>
            <Icon name={tab.kind === "ssh" ? "server" : tab.kind === "sftp" ? "folder" : "terminal"} size={13} />
            <span class="title">{tab.title}</span>
            <button
              class="close"
              aria-label="Sekmeyi kapat"
              onclick={(e) => {
                e.stopPropagation();
                store.close(tab.key);
              }}><Icon name="x" size={12} /></button
            >
          </div>
        {/each}
        <button class="newtab" title="Yeni yerel terminal" onclick={() => store.openLocal()}><Icon name="plus" size={14} /></button>
      </nav>

      <div class="panes">
        {#each store.tabs as tab (tab.key)}
          <div class="pane" class:hidden={tab.key !== store.activeKey}>
            <Terminal
              bind:this={terminals[tab.key]}
              {tab}
              active={tab.key === store.activeKey}
              onUpdate={(p) => store.patch(tab.key, p)}
            />
            <!-- Yalnızca SFTP: bağlıyken terminalin yerine tam ekran dosya tarayıcısı.
                 Bağlantı koparsa terminal (günlük ve "R ile yeniden bağlan") yeniden görünür. -->
            {#if tab.kind === "sftp" && tab.status === "open" && tab.termId}
              <div class="sftp-pane">
                {#key tab.termId}
                  <SftpPanel termId={tab.termId} wide />
                {/key}
              </div>
            {/if}
          </div>
        {/each}

        {#if !store.tabs.length}
          <div class="home">
            <div class="hero">
              <div class="big-logo"><Icon name="anchor" size={32} /></div>
              <h1>Liman</h1>
              <p>SSH, SFTP ve terminal için tek liman.</p>
            </div>
            <div class="actions">
              <button class="card" onclick={() => (editing = { session: null })}>
                <Icon name="server" size={22} />
                <strong>Yeni oturum</strong>
                <span>{mod}N</span>
              </button>
              <button class="card" onclick={() => store.openLocal()}>
                <Icon name="terminal" size={22} />
                <strong>Yerel terminal</strong>
                <span>{mod}T</span>
              </button>
            </div>
            {#if store.sessions.length}
              <h3>Kayıtlı oturumlar</h3>
              <div class="recent">
                {#each store.sessions.slice(0, 8) as s (s.id)}
                  <button onclick={() => store.openSession(s)}>
                    <Icon name="server" size={14} />
                    <span>{s.name}</span>
                  </button>
                {/each}
              </div>
            {/if}
            <button class="credit" onclick={() => (aboutOpen = true)}>© 2026 onuryrlmz · Hakkında</button>
          </div>
        {/if}
      </div>
    </section>
  </div>

  <footer class="status">
    <span>{describe(active)}</span>
    <span class="spacer"></span>
    {#each store.transfers as t (t.id)}
      <span class="xfer" class:err={t.state === "error"} title={t.error ?? t.name}>
        <Icon name={t.direction === "up" ? "upload" : "download"} size={12} />
        <span class="xname">{t.name}</span>
        {#if t.state === "error"}
          hata
        {:else if t.state === "done"}
          ✓
        {:else}
          <span class="bar"><span style:width="{t.total ? Math.round((t.done / t.total) * 100) : 0}%"></span></span>
          {formatSize(t.done)}{t.total ? ` / ${formatSize(t.total)}` : ""}
        {/if}
      </span>
    {/each}
  </footer>
</div>

{#if store.toast}
  <div class="toast" class:err={store.toast.kind === "error"}>{store.toast.text}</div>
{/if}
{#if editing}
  <SessionDialog session={editing.session} folder={editing.folder} onClose={() => (editing = null)} />
{/if}
{#if tunnelTab}
  <TunnelDialog tab={tunnelTab} onClose={() => (tunnelTab = null)} />
{/if}
{#if aboutOpen}
  <AboutDialog onClose={() => (aboutOpen = false)} />
{/if}
{#if menu}
  <ContextMenu {...menu} onClose={() => (menu = null)} />
{/if}

<style>
  :global(:root) {
    --bg: #0d1117;
    --panel: #161b22;
    --panel-2: #1c222b;
    --border: #2a313c;
    --hover: #1f2630;
    --text: #d6dde6;
    --muted: #8b95a3;
    --accent: #7fd1b9;
    --accent-soft: rgb(127 209 185 / 0.16);
    --danger: #ff7a85;
    --ok: #98c379;
    --warn: #e5c07b;
    color-scheme: dark;
  }
  :global(*) {
    box-sizing: border-box;
  }
  :global(html, body) {
    margin: 0;
    height: 100%;
    overflow: hidden;
    background: var(--bg);
    color: var(--text);
    font: 13px/1.4 -apple-system, BlinkMacSystemFont, "Segoe UI", Ubuntu, Roboto, sans-serif;
    -webkit-font-smoothing: antialiased;
    user-select: none;
  }
  :global(input, select, textarea) {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 7px 9px;
    outline: none;
    user-select: text;
  }
  :global(input:focus, select:focus, textarea:focus) {
    border-color: var(--accent);
  }
  :global(input[type="checkbox"]) {
    accent-color: var(--accent);
  }
  :global(code) {
    font-family: "JetBrains Mono", Menlo, Consolas, monospace;
    font-size: 12px;
  }
  :global(.btn) {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 7px 14px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel-2);
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  :global(.btn:hover:not(:disabled)) {
    border-color: #3a4350;
    background: var(--hover);
  }
  :global(.btn.primary) {
    background: var(--accent);
    border-color: var(--accent);
    color: #0b1714;
    font-weight: 600;
  }
  :global(.btn.primary:hover:not(:disabled)) {
    background: #95dcc7;
  }
  :global(.btn:disabled) {
    opacity: 0.5;
    cursor: default;
  }
  :global(.icon-btn) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--muted);
    cursor: pointer;
  }
  :global(.icon-btn:hover:not(:disabled)) {
    background: var(--hover);
    color: var(--text);
  }
  :global(.icon-btn:disabled) {
    opacity: 0.35;
    cursor: default;
  }
  :global(::-webkit-scrollbar) {
    width: 10px;
    height: 10px;
  }
  :global(::-webkit-scrollbar-thumb) {
    background: #2a313c;
    border-radius: 5px;
    border: 2px solid transparent;
    background-clip: padding-box;
  }

  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }

  /* Araç çubuğu */
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 10px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
  }
  .brand {
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    padding: 2px 4px;
    border-radius: 6px;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 7px;
    margin-right: 14px;
    font-weight: 700;
    letter-spacing: 0.2px;
  }
  .about-btn {
    margin-left: 6px;
  }
  .credit {
    margin-top: 8px;
    border: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 11.5px;
    cursor: pointer;
  }
  .credit:hover {
    color: var(--accent);
  }
  .logo {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    background: var(--accent);
    color: #0b1714;
    font: 700 11px "JetBrains Mono", Menlo, monospace;
  }
  .tool {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    min-width: 58px;
    padding: 4px 8px;
    border: 0;
    border-radius: 6px;
    background: none;
    color: var(--text);
    font-size: 11px;
    cursor: pointer;
  }
  .tool :global(svg) {
    color: var(--accent);
  }
  .tool:hover:not(:disabled) {
    background: var(--hover);
  }
  .tool.on {
    background: var(--accent-soft);
  }
  .tool:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .split {
    display: flex;
  }
  .tool.caret {
    min-width: 0;
    padding: 4px 3px;
    justify-content: center;
  }
  .tool.caret :global(svg) {
    color: var(--muted);
  }
  .quick {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
    width: min(320px, 35vw);
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: 7px;
    background: var(--bg);
    color: var(--muted);
  }
  .quick input {
    flex: 1;
    min-width: 0;
    border: 0;
    padding: 7px 0;
    background: none;
  }

  .main {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  /* Kenar çubuğu */
  .sidebar {
    display: flex;
    flex-direction: column;
    flex: none;
    background: var(--panel);
    min-width: 0;
  }
  .side-tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 6px 0;
    border-bottom: 1px solid var(--border);
  }
  .side-tabs button {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 7px 10px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 12.5px;
    cursor: pointer;
  }
  .side-tabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .side-body {
    flex: 1;
    min-height: 0;
  }
  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 40px 24px;
    color: var(--muted);
    text-align: center;
    font-size: 12.5px;
  }
  .resizer {
    width: 4px;
    flex: none;
    cursor: col-resize;
    background: var(--border);
    background-clip: content-box;
    padding-right: 3px;
  }
  .resizer:hover {
    background-color: var(--accent);
  }

  /* Sekmeler */
  .work {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .tabbar {
    display: flex;
    align-items: stretch;
    gap: 1px;
    height: 34px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 7px;
    max-width: 220px;
    padding: 0 6px 0 12px;
    color: var(--muted);
    font-size: 12.5px;
    cursor: default;
    border-right: 1px solid var(--border);
    white-space: nowrap;
  }
  .tab:hover {
    background: var(--hover);
  }
  .tab.on {
    background: #11151c;
    color: var(--text);
    box-shadow: inset 0 2px 0 var(--accent);
  }
  .tab .title {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .st {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    background: var(--muted);
  }
  .st.open {
    background: var(--ok);
  }
  .st.connecting {
    background: var(--warn);
  }
  .st.error {
    background: var(--danger);
  }
  .close {
    display: inline-flex;
    padding: 3px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--muted);
    cursor: pointer;
    opacity: 0;
  }
  .tab:hover .close,
  .tab.on .close {
    opacity: 1;
  }
  .close:hover {
    background: var(--border);
    color: var(--text);
  }
  .newtab {
    display: grid;
    place-items: center;
    width: 34px;
    border: 0;
    background: none;
    color: var(--muted);
    cursor: pointer;
  }
  .newtab:hover {
    color: var(--text);
    background: var(--hover);
  }
  .panes {
    position: relative;
    flex: 1;
    min-height: 0;
    background: #11151c;
  }
  .pane {
    position: absolute;
    inset: 0;
  }
  .sftp-pane {
    position: absolute;
    inset: 0;
    background: var(--panel);
  }
  .pane.hidden {
    visibility: hidden;
    pointer-events: none;
  }

  /* Karşılama ekranı */
  .home {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 20px;
    padding: 24px;
    overflow: auto;
  }
  .hero {
    text-align: center;
  }
  .big-logo {
    display: inline-grid;
    place-items: center;
    width: 64px;
    height: 64px;
    border-radius: 16px;
    background: var(--accent);
    color: #0b1714;
    font: 700 24px "JetBrains Mono", Menlo, monospace;
  }
  .hero h1 {
    margin: 12px 0 4px;
    font-size: 26px;
  }
  .hero p {
    margin: 0;
    color: var(--muted);
  }
  .actions {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    justify-content: center;
  }
  .card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    width: 180px;
    padding: 18px 12px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--panel);
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  .card :global(svg) {
    color: var(--accent);
  }
  .card:hover {
    border-color: var(--accent);
  }
  .card span {
    color: var(--muted);
    font-size: 11.5px;
  }
  .home h3 {
    margin: 8px 0 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.6px;
  }
  .recent {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    justify-content: center;
    max-width: 640px;
  }
  .recent button {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--panel);
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  .recent button :global(svg) {
    color: var(--accent);
  }
  .recent button:hover {
    border-color: var(--accent);
  }

  /* Durum çubuğu */
  .status {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 24px;
    padding: 0 10px;
    background: var(--panel);
    border-top: 1px solid var(--border);
    color: var(--muted);
    font-size: 11.5px;
    white-space: nowrap;
    overflow: hidden;
  }
  .spacer {
    flex: 1;
  }
  .xfer {
    display: flex;
    align-items: center;
    gap: 6px;
    font-variant-numeric: tabular-nums;
  }
  .xfer.err {
    color: var(--danger);
  }
  .xname {
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
  }
  .bar {
    width: 70px;
    height: 4px;
    border-radius: 2px;
    background: var(--border);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .toast {
    position: fixed;
    left: 50%;
    bottom: 40px;
    transform: translateX(-50%);
    z-index: 100;
    max-width: min(520px, calc(100vw - 32px));
    padding: 9px 16px;
    border-radius: 8px;
    background: var(--panel-2);
    border: 1px solid var(--border);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.4);
    font-size: 12.5px;
  }
  .toast.err {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
