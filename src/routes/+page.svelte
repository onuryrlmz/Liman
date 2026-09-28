<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import Terminal from "$lib/Terminal.svelte";
  import SessionList from "$lib/SessionList.svelte";
  import SftpPanel from "$lib/SftpPanel.svelte";
  import SessionDialog from "$lib/SessionDialog.svelte";
  import TunnelDialog from "$lib/TunnelDialog.svelte";
  import AboutDialog from "$lib/AboutDialog.svelte";
  import HostKeyDialog from "$lib/HostKeyDialog.svelte";
  import SettingsDialog from "$lib/SettingsDialog.svelte";
  import SnippetList from "$lib/SnippetList.svelte";
  import Palette, { type PaletteItem } from "$lib/Palette.svelte";
  import { settings } from "$lib/settings.svelte";
  import { updater } from "$lib/update-store.svelte";
  import ContextMenu, { type MenuItem } from "$lib/ContextMenu.svelte";
  import Icon from "$lib/Icon.svelte";
  import { api, formatSize, parseQuick, type Session, type Transfer } from "$lib/api";
  import { store, computeLayout, leaves, type Tab, type View, type Rect, type SplitDir } from "$lib/tabs.svelte";

  const isMac = navigator.platform.toLowerCase().includes("mac");
  const mod = isMac ? "⌘" : "Ctrl+Shift+";

  let sidebarTab = $state<"sessions" | "sftp" | "snippets">("sessions");
  let paletteOpen = $state(false);
  let sidebarOpen = $state(true);
  let sidebarWidth = $state(loadWidth());
  let editing = $state<{ session: Session | null; folder?: string | null } | null>(null);
  let tunnelTab = $state<Tab | null>(null);
  let aboutOpen = $state(false);
  let settingsOpen = $state(false);
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
    store.loadSnippets();
    settings.load().then(() => {
      if (settings.value.checkUpdates) updater.check(true);
    });
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

  function tabMenu(ev: MouseEvent, view: View) {
    ev.preventDefault();
    store.activeKey = view.key;
    const tab = store.pane(view.focus);
    if (!tab) return;
    const many = leaves(view.root).length > 1;
    const items: (MenuItem | null)[] = [
      { label: `Sağa böl (${splitKeys.row})`, icon: "splitRow", action: () => store.split("row", view) },
      { label: `Aşağı böl (${splitKeys.col})`, icon: "splitCol", action: () => store.split("col", view) },
      null,
      { label: "Yeniden bağlan", icon: "refresh", disabled: tab.status === "open" || tab.status === "connecting", action: () => terminals[tab.key]?.restart() },
      { label: "Yeni sekmede çoğalt", icon: "copy", action: () => store.duplicate(tab) },
    ];
    if (tab.kind !== "local") {
      items.push({ label: "Tüneller…", icon: "tunnel", disabled: tab.status !== "open", action: () => (tunnelTab = tab) });
    }
    items.push(null);
    if (many) items.push({ label: "Bu panoyu kapat", icon: "x", action: () => store.closePane(tab.key) });
    items.push(
      { label: many ? "Sekmeyi kapat" : "Kapat", icon: many ? undefined : "x", action: () => store.close(view.key) },
      { label: "Diğer sekmeleri kapat", action: () => store.views.filter((v) => v.key !== view.key).forEach((v) => store.close(v.key)) },
    );
    menu = { x: ev.clientX, y: ev.clientY, items };
  }

  const paletteKey = isMac ? "⌘P" : "Ctrl+Shift+P";

  function paletteItems(): PaletteItem[] {
    const connected = store.active && store.active.kind !== "sftp" && store.active.status === "open";
    const items: PaletteItem[] = [];
    if (connected) {
      for (const sn of store.snippets) {
        items.push({ kind: "Parçacık", icon: "bolt", label: sn.name, detail: sn.command.split("\n")[0], action: () => store.runSnippet(sn) });
      }
    }
    for (const se of store.sessions) {
      items.push({
        kind: se.kind === "sftp" ? "SFTP" : "Bağlan",
        icon: se.kind === "sftp" ? "folder" : "server",
        label: se.name,
        detail: `${se.username ? se.username + "@" : ""}${se.host}${se.folder ? " · " + se.folder : ""}`,
        action: () => store.openSession(se),
      });
    }
    for (const v of store.views) {
      const p = store.pane(v.focus);
      if (p) items.push({ kind: "Sekme", icon: "terminal", label: p.title, action: () => (store.activeKey = v.key) });
    }
    const act = (label: string, icon: string, action: () => void) => items.push({ kind: "Komut", icon, label, action });
    act("Yeni yerel terminal", "terminal", () => store.openLocal());
    act("Yeni oturum", "plus", () => (editing = { session: null }));
    if (store.active) {
      act("Sağa böl", "splitRow", () => store.split("row"));
      act("Aşağı böl", "splitCol", () => store.split("col"));
    }
    act(store.broadcast === "off" ? "MultiExec'i aç (bu sekme)" : "MultiExec'i kapat", "broadcast", () => toggleBroadcast(store.broadcast === "off" ? "tab" : (store.broadcast as "tab" | "all")));
    act("Parçacıkları göster", "bolt", () => ((sidebarTab = "snippets"), (sidebarOpen = true)));
    act("Ayarlar", "gear", () => (settingsOpen = true));
    act("Hakkında", "info", () => (aboutOpen = true));
    return items;
  }

  function toggleBroadcast(scope: "tab" | "all") {
    store.broadcast = store.broadcast === scope ? "off" : scope;
    if (store.broadcast !== "off") {
      store.notify(scope === "all" ? "MultiExec: yazdıklarınız TÜM sekmelere gidiyor" : "MultiExec: yazdıklarınız bu sekmedeki tüm panolara gidiyor");
    }
  }

  const broadcastKeys = isMac ? { tab: "⇧⌘I", all: "⌥⇧⌘I" } : { tab: "Ctrl+Shift+I", all: "Ctrl+Alt+Shift+I" };

  const splitKeys: Record<SplitDir, string> = isMac ? { row: "⌘D", col: "⇧⌘D" } : { row: "Ctrl+Shift+D", col: "Ctrl+Shift+E" };

  // Bölünmüş panolar: konumlar yerleşim ağacından hesaplanır. Terminal bileşenleri
  // düz bir listede kalır ki bölme/kapama sırasında yeniden oluşturulmasınlar.
  const layouts = $derived(new Map(store.views.map((v) => [v.key, computeLayout(v.root)])));
  function paneRect(key: string): { view: View | null; rect: Rect } {
    const view = store.viewOf(key);
    return { view, rect: (view && layouts.get(view.key)?.panes.get(key)) || { x: 0, y: 0, w: 1, h: 1 } };
  }
  let panesEl = $state<HTMLDivElement>();

  function dragDivider(e: PointerEvent, node: Extract<import("$lib/tabs.svelte").Layout, { type: "split" }>, area: Rect) {
    e.preventDefault();
    const box = panesEl!.getBoundingClientRect();
    const move = (ev: PointerEvent) => {
      const pos =
        node.dir === "row"
          ? ((ev.clientX - box.left) / box.width - area.x) / area.w
          : ((ev.clientY - box.top) / box.height - area.y) / area.h;
      node.ratio = Math.max(0.1, Math.min(0.9, pos));
      // Nesne yerinde değişti; türetilmiş yerleşimi tazele.
      const v = store.activeView;
      if (v) v.root = { ...v.root };
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function onKey(e: KeyboardEvent) {
    const primary = isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && e.shiftKey;
    if (e.ctrlKey && e.key === "Tab") {
      store.cycle(e.shiftKey ? -1 : 1);
    } else if (primary && e.code === "KeyT") {
      store.openLocal();
    } else if (primary && e.code === "KeyW") {
      const v = store.activeView;
      if (v) store.closePane(v.focus);
    } else if (isMac ? e.metaKey && !e.ctrlKey && e.code === "KeyD" : e.ctrlKey && e.shiftKey && e.code === "KeyD") {
      store.split(isMac && e.shiftKey ? "col" : "row");
    } else if (!isMac && e.ctrlKey && e.shiftKey && e.code === "KeyE") {
      store.split("col");
    } else if (primary && e.code === "KeyP") {
      paletteOpen = !paletteOpen;
    } else if (primary && e.code === "KeyI") {
      toggleBroadcast(e.altKey ? "all" : "tab");
    } else if (primary && (e.code === "BracketRight" || e.code === "BracketLeft")) {
      store.focusNext(e.code === "BracketRight" ? 1 : -1);
    } else if (primary && e.code === "KeyN") {
      editing = { session: null };
    } else if (primary && e.code === "KeyB") {
      sidebarOpen = !sidebarOpen;
    } else if (zoomMod(e) && (e.code === "Equal" || e.code === "NumpadAdd")) {
      settings.zoom(1);
    } else if (zoomMod(e) && (e.code === "Minus" || e.code === "NumpadSubtract")) {
      settings.zoom(-1);
    } else if (zoomMod(e) && (e.code === "Digit0" || e.code === "Numpad0")) {
      settings.zoom(null);
    } else if (zoomMod(e) && e.code === "Comma") {
      settingsOpen = true;
    } else if (isMac && e.metaKey && /^Digit[1-9]$/.test(e.code)) {
      const v = store.views[Number(e.code.slice(5)) - 1];
      if (v) store.activeKey = v.key;
    } else return;
    e.preventDefault();
    e.stopPropagation();
  }

  // Yakınlaştırma ve ayarlar: macOS'ta ⌘, diğerlerinde Ctrl (Shift'siz).
  function zoomMod(e: KeyboardEvent) {
    return isMac ? e.metaKey && !e.ctrlKey && !e.altKey : e.ctrlKey && !e.metaKey && !e.altKey;
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
    <div class="split">
      <button class="tool" disabled={!active} onclick={() => store.split("row")} title="Sağa böl ({splitKeys.row})">
        <Icon name="splitRow" size={18} /><span>Böl</span>
      </button>
      <button
        class="tool caret"
        disabled={!active}
        title="Bölme yönü"
        onclick={(ev) => {
          const r = (ev.currentTarget as HTMLElement).getBoundingClientRect();
          menu = {
            x: r.left,
            y: r.bottom + 4,
            items: [
              { label: `Sağa böl (${splitKeys.row})`, icon: "splitRow", action: () => store.split("row") },
              { label: `Aşağı böl (${splitKeys.col})`, icon: "splitCol", action: () => store.split("col") },
            ],
          };
        }}><Icon name="chevronDown" size={13} /></button
      >
    </div>
    <div class="split">
      <button
        class="tool"
        class:on={store.broadcast !== "off"}
        class:warn={store.broadcast !== "off"}
        onclick={() => toggleBroadcast("tab")}
        title="MultiExec: bu sekmedeki tüm panolara yaz ({broadcastKeys.tab})"
      >
        <Icon name="broadcast" size={18} /><span>{store.broadcast === "all" ? "Tümüne" : "MultiExec"}</span>
      </button>
      <button
        class="tool caret"
        title="MultiExec kapsamı"
        onclick={(ev) => {
          const r = (ev.currentTarget as HTMLElement).getBoundingClientRect();
          menu = {
            x: r.left,
            y: r.bottom + 4,
            items: [
              { label: `Bu sekmedeki panolar (${broadcastKeys.tab})`, icon: store.broadcast === "tab" ? "check" : undefined, action: () => toggleBroadcast("tab") },
              { label: `Tüm sekmeler (${broadcastKeys.all})`, icon: store.broadcast === "all" ? "check" : undefined, action: () => toggleBroadcast("all") },
              null,
              { label: "Kapat", disabled: store.broadcast === "off", action: () => (store.broadcast = "off") },
            ],
          };
        }}><Icon name="chevronDown" size={13} /></button
      >
    </div>
    <button class="tool" class:on={sidebarOpen} onclick={() => (sidebarOpen = !sidebarOpen)} title="Kenar çubuğu ({mod}B)">
      <Icon name="sidebar" size={18} /><span>Panel</span>
    </button>
    <form class="quick" onsubmit={quickConnect}>
      <Icon name="bolt" size={14} />
      <input bind:value={quick} placeholder="Hızlı bağlan: kullanıcı@sunucu:port  (sftp://… yalnızca SFTP)" spellcheck="false" autocapitalize="off" />
    </form>
    <button class="icon-btn about-btn" onclick={() => (settingsOpen = true)} title="Ayarlar ({isMac ? '⌘' : 'Ctrl+'},)" aria-label="Ayarlar">
      <Icon name="gear" size={17} />
    </button>
    <button class="icon-btn" onclick={() => (aboutOpen = true)} title="Hakkında" aria-label="Hakkında">
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
          <button class:on={sidebarTab === "snippets"} onclick={() => (sidebarTab = "snippets")} title="Komut parçacıkları">
            <Icon name="bolt" size={14} /> Parçacık
          </button>
        </div>
        <div class="side-body">
          {#if sidebarTab === "sessions"}
            <SessionList onEdit={(s, folder) => (editing = { session: s, folder })} />
          {:else if sidebarTab === "snippets"}
            <SnippetList />
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
        {#each store.views as view (view.key)}
          {@const tab = store.pane(view.focus)}
          {@const count = leaves(view.root).length}
          {#if tab}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
            <div
              class="tab"
              class:on={view.key === store.activeKey}
              onclick={() => (store.activeKey = view.key)}
              onauxclick={(e) => e.button === 1 && store.close(view.key)}
              oncontextmenu={(e) => tabMenu(e, view)}
              title={describe(tab)}
            >
              <span class="st {tab.status}"></span>
              <Icon name={tab.kind === "ssh" ? "server" : tab.kind === "sftp" ? "folder" : "terminal"} size={13} />
              <span class="title">{tab.title}</span>
              {#if count > 1}<span class="count" title="{count} pano">{count}</span>{/if}
              <button
                class="close"
                aria-label="Sekmeyi kapat"
                onclick={(e) => {
                  e.stopPropagation();
                  store.close(view.key);
                }}><Icon name="x" size={12} /></button
              >
            </div>
          {/if}
        {/each}
        <button class="newtab" title="Yeni yerel terminal" onclick={() => store.openLocal()}><Icon name="plus" size={14} /></button>
      </nav>

      <div class="panes" bind:this={panesEl}>
        {#each store.panes as tab (tab.key)}
          {@const { view, rect } = paneRect(tab.key)}
          {@const visible = !!view && view.key === store.activeKey}
          {@const split = !!view && view.root.type === "split"}
          <div
            class="pane"
            class:hidden={!visible}
            class:split
            class:focused={split && view?.focus === tab.key}
            class:broadcast={store.inBroadcast(tab.key) && tab.kind !== "sftp"}
            style:left="{rect.x * 100}%"
            style:top="{rect.y * 100}%"
            style:width="{rect.w * 100}%"
            style:height="{rect.h * 100}%"
          >
            <Terminal
              bind:this={terminals[tab.key]}
              {tab}
              {visible}
              focused={view?.focus === tab.key}
              onUpdate={(p) => store.patch(tab.key, p)}
              onFocus={() => store.focusPane(tab.key)}
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

        {#if store.activeView}
          {#each layouts.get(store.activeView.key)?.dividers ?? [] as d}
            {@const r = d.node.dir === "row"}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
              class="divider"
              class:row={r}
              class:col={!r}
              style:left="{(r ? d.area.x + d.area.w * d.node.ratio : d.area.x) * 100}%"
              style:top="{(r ? d.area.y : d.area.y + d.area.h * d.node.ratio) * 100}%"
              style:width={r ? null : `${d.area.w * 100}%`}
              style:height={r ? `${d.area.h * 100}%` : null}
              onpointerdown={(e) => dragDivider(e, d.node, d.area)}
            ></div>
          {/each}
        {/if}

        {#if !store.views.length}
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
    {#if !updater.dismissed && (updater.state.kind === "available" || updater.state.kind === "downloading" || updater.state.kind === "ready")}
      <span class="update">
        {#if updater.state.kind === "available"}
          Liman {updater.state.version} çıktı
          <button onclick={() => updater.install()}>Kur</button>
          <button class="x" title="Sonra" onclick={() => (updater.dismissed = true)}>×</button>
        {:else if updater.state.kind === "downloading"}
          Güncelleme indiriliyor… {updater.state.total ? Math.round((updater.state.done / updater.state.total) * 100) : 0}%
        {:else}
          Güncelleme hazır
          <button onclick={() => updater.restart()}>Yeniden başlat</button>
        {/if}
      </span>
    {/if}
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
<HostKeyDialog />
{#if paletteOpen}
  <Palette items={paletteItems()} onClose={() => (paletteOpen = false)} />
{/if}
{#if settingsOpen}
  <SettingsDialog onClose={() => (settingsOpen = false)} />
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
    overflow: hidden;
  }
  .pane.split {
    /* Bölücü için boşluk */
    border: 1px solid transparent;
  }
  .pane.focused {
    border-color: rgb(127 209 185 / 0.35);
  }
  .pane.broadcast {
    border: 1px solid rgb(229 192 123 / 0.55);
  }
  .pane.broadcast.focused {
    border-color: var(--warn);
  }
  .tool.warn {
    background: rgb(229 192 123 / 0.16);
  }
  .tool.warn :global(svg) {
    color: var(--warn);
  }
  .divider {
    position: absolute;
    z-index: 4;
  }
  .divider.row {
    width: 7px;
    margin-left: -3px;
    cursor: col-resize;
  }
  .divider.col {
    height: 7px;
    margin-top: -3px;
    cursor: row-resize;
  }
  .divider::after {
    content: "";
    position: absolute;
    background: var(--border);
  }
  .divider.row::after {
    left: 3px;
    top: 0;
    bottom: 0;
    width: 1px;
  }
  .divider.col::after {
    top: 3px;
    left: 0;
    right: 0;
    height: 1px;
  }
  .divider:hover::after {
    background: var(--accent);
  }
  .tab .count {
    min-width: 16px;
    padding: 0 4px;
    border-radius: 8px;
    background: var(--border);
    color: var(--text);
    font-size: 10px;
    text-align: center;
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
  .update {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--accent);
  }
  .update button {
    padding: 0 7px;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
  }
  .update button.x {
    border: 0;
    color: var(--muted);
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
