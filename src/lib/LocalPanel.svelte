<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import { join } from "@tauri-apps/api/path";
  import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
  import Icon from "./Icon.svelte";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import { api, errText, formatDate, formatSize, type Entry } from "./api";
  import { store } from "./tabs.svelte";
  import { fileDrag } from "./file-drag.svelte";
  import SyncDialog from "./SyncDialog.svelte";

  /** Çift panelli SFTP'nin yerel tarafı. `path` üst bileşenle paylaşılır (indirme hedefi). */
  let { termId, path = $bindable("") }: { termId: string; path?: string } = $props();

  let pathInput = $state("");
  let parent = $state<string | null>(null);
  let entries = $state<Entry[]>([]);
  let selected = $state<Set<string>>(new Set());
  let showHidden = $state(false);
  let error = $state("");
  let menu = $state<{ x: number; y: number; items: (MenuItem | null)[] } | null>(null);
  let renaming = $state<string | null>(null);
  let renameValue = $state("");
  let creating = $state(false);
  let newName = $state("");
  let syncing = $state(false);
  const remoteDir = $derived(store.sftpPaths[termId]);

  const visible = $derived(showHidden ? entries : entries.filter((e) => !e.name.startsWith(".")));
  const selectedEntries = $derived(entries.filter((e) => selected.has(e.path)));

  async function load(p: string) {
    error = "";
    try {
      const l = await api.localList(p);
      entries = l.entries;
      parent = l.parent;
      if (l.path !== path) selected = new Set();
      path = pathInput = l.path;
    } catch (e) {
      error = errText(e);
      pathInput = path;
    }
  }
  load(path);

  // İndirme bittiğinde listeyi tazele.
  let doneCount = 0;
  $effect(() => {
    const n = store.transfers.filter((t) => t.state === "done" && t.direction === "down").length;
    if (n !== doneCount) {
      doneCount = n;
      if (n) load(path);
    }
  });

  // Uzak panelden bırakılanları bu klasöre indir.
  $effect(() =>
    fileDrag.zone("local", (items) => {
      const dir = path;
      for (const e of items) {
        const id = crypto.randomUUID();
        store.upsertTransfer({ id, name: e.name, done: 0, total: 0, state: "run", direction: "down" });
        join(dir, e.name).then((local) => api.sftpDownload(termId, e.path, local, id).catch(() => {}));
      }
    }),
  );

  function upload(items: Entry[]) {
    fileDrag.deliver("remote", items, "local");
  }

  async function remove(items: Entry[]) {
    if (!items.length) return;
    const what = items.length === 1 ? `"${items[0].name}"` : `${items.length} öğe`;
    if (!(await ask(`${what} bu bilgisayardan kalıcı olarak silinsin mi?`, { kind: "warning", title: "Sil" }))) return;
    for (const e of items) {
      try {
        await api.localRemove(e.path);
      } catch (err) {
        store.notify(`${e.name}: ${errText(err)}`, "error");
      }
    }
    load(path);
  }

  async function commitRename(e: Entry) {
    const name = renameValue.trim();
    renaming = null;
    if (!name || name === e.name) return;
    try {
      await api.localRename(e.path, name);
      load(path);
    } catch (err) {
      store.notify(errText(err), "error");
    }
  }

  async function commitMkdir() {
    const name = newName.trim();
    if (!creating) return;
    creating = false;
    if (!name) return;
    try {
      await api.localMkdir(path, name);
      load(path);
    } catch (err) {
      store.notify(errText(err), "error");
    }
  }

  function activate(e: Entry) {
    if (e.isDir) load(e.path);
    else openPath(e.path).catch((err) => store.notify(errText(err), "error"));
  }

  let lastClicked: string | null = null;
  function click(ev: MouseEvent, e: Entry) {
    if (ev.metaKey || ev.ctrlKey) {
      const s = new Set(selected);
      s.has(e.path) ? s.delete(e.path) : s.add(e.path);
      selected = s;
    } else if (ev.shiftKey && lastClicked) {
      const i = visible.findIndex((x) => x.path === lastClicked);
      const j = visible.findIndex((x) => x.path === e.path);
      const [a, b] = i < j ? [i, j] : [j, i];
      selected = new Set(visible.slice(a, b + 1).map((x) => x.path));
    } else {
      selected = new Set([e.path]);
    }
    lastClicked = e.path;
  }

  function context(ev: MouseEvent, e: Entry | null) {
    ev.preventDefault();
    if (e && !selected.has(e.path)) selected = new Set([e.path]);
    const items = e ? selectedEntries : [];
    const single = items.length === 1 ? items[0] : null;
    menu = {
      x: ev.clientX,
      y: ev.clientY,
      items: e
        ? [
            { label: "Sunucuya yükle", icon: "upload", action: () => upload(items) },
            { label: single?.isDir ? "Aç" : "Varsayılan uygulamayla aç", icon: single?.isDir ? "folder" : "file", disabled: !single, action: () => single && activate(single) },
            { label: "Klasörde göster", icon: "folder", disabled: !single, action: () => single && revealItemInDir(single.path) },
            null,
            { label: "Yeniden adlandır", icon: "edit", disabled: !single, action: () => single && ((renaming = single.path), (renameValue = single.name)) },
            null,
            { label: "Sil", icon: "trash", danger: true, action: () => remove(items) },
          ]
        : [
            { label: "Yeni klasör", icon: "folderPlus", action: () => ((newName = ""), (creating = true)) },
            { label: "Yenile", icon: "refresh", action: () => load(path) },
            { label: showHidden ? "Gizli dosyaları gizle" : "Gizli dosyaları göster", action: () => (showHidden = !showHidden) },
          ],
    };
  }

  function onKey(ev: KeyboardEvent) {
    if (renaming || creating) return;
    if (ev.key === "Delete" || (ev.key === "Backspace" && ev.metaKey)) remove(selectedEntries);
    else if (ev.key === "Enter" && selectedEntries.length === 1) activate(selectedEntries[0]);
    else if (ev.key === "Backspace" && parent) load(parent);
    else if (ev.key === "F2" && selectedEntries.length === 1) ((renaming = selectedEntries[0].path), (renameValue = selectedEntries[0].name));
  }
</script>

<div class="local" class:over={fileDrag.drag?.over === "local"} data-file-drop="local">
  <div class="toolbar">
    <span class="side">Bu bilgisayar</span>
    <button class="icon-btn" title="Üst klasör" disabled={!parent} onclick={() => parent && load(parent)}><Icon name="up" /></button>
    <button class="icon-btn" title="Ev klasörü" onclick={() => load("")}><Icon name="home" /></button>
    <button class="icon-btn" title="Yenile" onclick={() => load(path)}><Icon name="refresh" /></button>
    <span class="spacer"></span>
    <button class="icon-btn" title="Yeni klasör" onclick={() => ((newName = ""), (creating = true))}><Icon name="folderPlus" /></button>
    <button class="icon-btn" title="Sil" disabled={!selectedEntries.length} onclick={() => remove(selectedEntries)}><Icon name="trash" /></button>
    <button class="btn send" title="Bu klasörü sunucudaki açık klasörle karşılaştır" disabled={!remoteDir} onclick={() => (syncing = true)}>
      <Icon name="refresh" size={13} /> Eşitle
    </button>
    <button class="btn send" title="Seçilenleri sunucuya yükle" disabled={!selectedEntries.length} onclick={() => upload(selectedEntries)}>
      Yükle <Icon name="chevron" size={13} />
    </button>
  </div>
  <form
    class="path"
    onsubmit={(e) => {
      e.preventDefault();
      load(pathInput.trim());
    }}
  >
    <input bind:value={pathInput} spellcheck="false" aria-label="Yerel klasör" />
  </form>
  <label class="hidden-toggle"><input type="checkbox" bind:checked={showHidden} /> Gizli dosyaları göster</label>

  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="list" tabindex="0" onkeydown={onKey} oncontextmenu={(e) => context(e, null)} role="listbox" aria-label="Yerel dosyalar">
    <div class="row head" aria-hidden="true">
      <span class="ic"></span>
      <span class="name">Ad</span>
      <span class="size">Boyut</span>
      <span class="date">Değiştirilme</span>
    </div>
    {#if error}<p class="err">{error}</p>{/if}
    {#if creating}
      <div class="row">
        <span class="ic dir"><Icon name="folder" size={15} /></span>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="rename"
          bind:value={newName}
          placeholder="Yeni klasör adı"
          autofocus
          onkeydown={(ev) => {
            ev.stopPropagation();
            if (ev.key === "Enter") commitMkdir();
            if (ev.key === "Escape") creating = false;
          }}
          onblur={commitMkdir}
        />
      </div>
    {/if}
    {#each visible as e (e.path)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="row"
        class:sel={selected.has(e.path)}
        role="option"
        aria-selected={selected.has(e.path)}
        tabindex="-1"
        onclick={(ev) => click(ev, e)}
        onpointerdown={(ev) => {
          if (renaming) return;
          if (!selected.has(e.path) && !ev.metaKey && !ev.ctrlKey && !ev.shiftKey) selected = new Set([e.path]);
          fileDrag.start(ev, "local", () => selectedEntries);
        }}
        ondblclick={() => activate(e)}
        oncontextmenu={(ev) => {
          ev.stopPropagation();
          context(ev, e);
        }}
        title={e.name}
      >
        <span class="ic" class:dir={e.isDir}><Icon name={e.isDir ? "folder" : e.isLink ? "link" : "file"} size={15} /></span>
        {#if renaming === e.path}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="rename"
            bind:value={renameValue}
            autofocus
            onclick={(ev) => ev.stopPropagation()}
            onkeydown={(ev) => {
              ev.stopPropagation();
              if (ev.key === "Enter") commitRename(e);
              if (ev.key === "Escape") renaming = null;
            }}
            onblur={() => commitRename(e)}
          />
        {:else}
          <span class="name">{e.name}</span>
        {/if}
        <span class="size">{e.isDir ? "" : formatSize(e.size)}</span>
        <span class="date">{formatDate(e.mtime)}</span>
      </div>
    {:else}
      {#if !error}<p class="empty">Klasör boş</p>{/if}
    {/each}
  </div>
  {#if fileDrag.drag?.over === "local"}
    <div class="drop-hint"><Icon name="download" size={22} /> {path} klasörüne indir</div>
  {/if}
</div>

{#if syncing && remoteDir}
  <SyncDialog {termId} local={path} remote={remoteDir} onClose={() => ((syncing = false), load(path))} />
{/if}
{#if menu}
  <ContextMenu {...menu} onClose={() => (menu = null)} />
{/if}

<style>
  .local {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--panel);
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 14px 4px;
  }
  .side {
    margin-right: 6px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--muted);
  }
  .spacer {
    flex: 1;
  }
  .send {
    margin-left: 6px;
    padding: 3px 8px;
    font-size: 12px;
  }
  .path {
    padding: 2px 14px 4px;
  }
  .path input {
    width: 100%;
    font-family: "JetBrains Mono", Menlo, Consolas, monospace;
    font-size: 11.5px;
    padding: 5px 7px;
  }
  .hidden-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 16px 6px;
    font-size: 11.5px;
    color: var(--muted);
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 0 8px 12px;
    outline: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 4px 10px;
    border-radius: 4px;
    font-size: 13px;
    cursor: default;
    user-select: none;
    white-space: nowrap;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.sel {
    background: var(--accent-soft);
  }
  .row.head {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--panel);
    color: var(--muted);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    border-bottom: 1px solid var(--border);
    border-radius: 0;
  }
  .ic {
    display: inline-flex;
    width: 15px;
    color: var(--muted);
    flex: none;
  }
  .ic.dir {
    color: #e5c07b;
  }
  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .size,
  .date {
    color: var(--muted);
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
  }
  .size {
    width: 80px;
    text-align: right;
  }
  .date {
    width: 130px;
    padding-left: 16px;
  }
  .rename {
    flex: 1;
    padding: 1px 4px;
    font-size: 12.5px;
  }
  .empty,
  .err {
    padding: 10px;
    font-size: 12.5px;
    color: var(--muted);
  }
  .err {
    color: var(--danger);
  }
  .over .list {
    opacity: 0.4;
  }
  .drop-hint {
    position: absolute;
    inset: 90px 10px 10px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border: 2px dashed var(--accent);
    border-radius: 10px;
    color: var(--accent);
    font-size: 12.5px;
    text-align: center;
    padding: 12px;
    pointer-events: none;
    word-break: break-all;
  }
</style>
