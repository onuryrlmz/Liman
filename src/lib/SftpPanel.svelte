<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { open, ask } from "@tauri-apps/plugin-dialog";
  import { join } from "@tauri-apps/api/path";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import Icon from "./Icon.svelte";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import EditorDialog from "./EditorDialog.svelte";
  import { api, errText, formatDate, formatSize, joinPath, parentPath, type Entry } from "./api";
  import { store } from "./tabs.svelte";
  import { settings } from "./settings.svelte";
  import { openPath } from "@tauri-apps/plugin-opener";
  import { fileDrag } from "./file-drag.svelte";

  /** wide: ana alanda tam ekran (yalnızca SFTP sekmesi); tarih ve izin sütunları da görünür. */
  /** localDir: çift panelde yerel klasör; indirmeler oraya gider ve sürükle-bırak açılır. */
  let {
    termId,
    wide = false,
    localDir = null,
  }: { termId: string; wide?: boolean; localDir?: string | null } = $props();

  let path = $state(store.sftpPaths[termId] ?? "");
  let pathInput = $state("");
  let entries = $state<Entry[]>([]);
  let selected = $state<Set<string>>(new Set());
  let loading = $state(false);
  let error = $state("");
  let showHidden = $state(false);
  let menu = $state<{ x: number; y: number; items: (MenuItem | null)[] } | null>(null);
  let editing = $state<string | null>(null);
  let dropping = $state(false);
  let renaming = $state<string | null>(null);
  let renameValue = $state("");
  let creating = $state(false);
  let newName = $state("");
  let panel: HTMLDivElement;
  let unlistenDrop: (() => void) | undefined;

  const visible = $derived(showHidden ? entries : entries.filter((e) => !e.name.startsWith(".")));
  const selectedEntries = $derived(entries.filter((e) => selected.has(e.path)));

  async function load(p: string) {
    loading = true;
    error = "";
    try {
      const list = await api.sftpList(termId, p);
      entries = list;
      path = p;
      pathInput = p;
      selected = new Set();
      store.sftpPaths[termId] = p;
    } catch (e) {
      error = errText(e);
      pathInput = path;
    } finally {
      loading = false;
    }
  }

  onMount(async () => {
    try {
      await load(path || (await api.sftpHome(termId)));
    } catch (e) {
      error = errText(e);
    }

    // İşletim sisteminden sürüklenen dosyaları yükle.
    unlistenDrop = await getCurrentWebview().onDragDropEvent((ev) => {
      const r = panel?.getBoundingClientRect();
      const inside = (pos: { x: number; y: number }) => {
        if (!r) return false;
        const x = pos.x / devicePixelRatio;
        const y = pos.y / devicePixelRatio;
        return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
      };
      if (ev.payload.type === "over") dropping = inside(ev.payload.position);
      else if (ev.payload.type === "leave") dropping = false;
      else if (ev.payload.type === "drop") {
        const hit = inside(ev.payload.position);
        dropping = false;
        if (hit) uploadPaths(ev.payload.paths);
      }
    });
  });

  onDestroy(() => unlistenDrop?.());

  // Çift panel: yerel panelden bırakılanları yükle.
  $effect(() => {
    if (localDir === null) return;
    return fileDrag.zone("remote", (items) => uploadPaths(items.map((e) => e.path)));
  });

  // Başka yerden (eşitleme, yerel panel) yapılan yüklemeler bitince listeyi tazele.
  let upDone = 0;
  $effect(() => {
    const n = store.transfers.filter((t) => t.state === "done" && t.direction !== "down").length;
    if (n !== upDone) {
      upDone = n;
      if (n && path) load(path);
    }
  });

  /** Yerel panelin "→" düğmesi için dışarıdan çağrılır. */
  export function uploadFromLocal(paths: string[]) {
    return uploadPaths(paths);
  }

  export function downloadSelected() {
    return download(selectedEntries);
  }

  function transferId() {
    return crypto.randomUUID();
  }

  async function uploadPaths(paths: string[]) {
    const dir = path;
    await Promise.all(
      paths.map(async (p) => {
        const id = transferId();
        const name = p.split(/[\\/]/).pop() ?? p;
        store.upsertTransfer({ id, name, done: 0, total: 0, state: "run", direction: "up" });
        try {
          await api.sftpUpload(termId, p, dir, id);
        } catch {
          /* hata aktarım satırında gösteriliyor */
        }
      }),
    );
    if (path === dir) load(dir);
  }

  async function uploadFiles(directory = false) {
    const res = await open({ multiple: !directory, directory, title: directory ? "Yüklenecek klasör" : "Yüklenecek dosyalar" });
    if (!res) return;
    uploadPaths(Array.isArray(res) ? res : [res]);
  }

  async function download(items: Entry[], target: string | null = localDir) {
    if (!items.length) return;
    const dest = target ?? (await open({ directory: true, title: "İndirilecek klasörü seçin" }));
    if (typeof dest !== "string") return;
    for (const e of items) {
      const id = transferId();
      store.upsertTransfer({ id, name: e.name, done: 0, total: 0, state: "run", direction: "down" });
      join(dest, e.name).then((local) => api.sftpDownload(termId, e.path, local, id).catch(() => {}));
    }
  }

  function mkdir() {
    newName = "";
    creating = true;
  }

  async function commitMkdir() {
    const name = newName.trim();
    if (!creating) return;
    creating = false;
    if (!name) return;
    try {
      await api.sftpMkdir(termId, joinPath(path, name));
      await load(path);
    } catch (e) {
      store.notify(errText(e), "error");
    }
  }

  async function remove(items: Entry[]) {
    if (!items.length) return;
    const what = items.length === 1 ? `"${items[0].name}"` : `${items.length} öğe`;
    if (!(await ask(`${what} kalıcı olarak silinsin mi?`, { kind: "warning", title: "Sil" }))) return;
    for (const e of items) {
      try {
        await api.sftpRemove(termId, e.path, e.isDir);
      } catch (err) {
        store.notify(`${e.name}: ${errText(err)}`, "error");
      }
    }
    load(path);
  }

  function startRename(e: Entry) {
    renaming = e.path;
    renameValue = e.name;
  }

  async function commitRename(e: Entry) {
    const name = renameValue.trim();
    renaming = null;
    if (!name || name === e.name) return;
    try {
      await api.sftpRename(termId, e.path, joinPath(path, name));
      load(path);
    } catch (err) {
      store.notify(errText(err), "error");
    }
  }

  function activate(e: Entry) {
    if (e.isDir) load(e.path);
    else if (settings.value.openFilesWith === "external") editLocal(e);
    else editing = e.path;
  }

  /** Dosyayı bilgisayardaki editörde açar; kaydedildikçe sunucuya yüklenir. */
  async function editLocal(e: Entry) {
    try {
      const local = await api.sftpEditLocal(termId, e.path);
      const app = settings.value.externalEditor.trim() || undefined;
      await openPath(local, app);
      store.notify(`${e.name} açıldı; kaydettikçe sunucuya yüklenecek`);
    } catch (err) {
      store.notify(errText(err), "error");
    }
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
    const fileItems: (MenuItem | null)[] = [];
    if (single?.isDir) fileItems.push({ label: "Aç", icon: "folder", action: () => load(single.path) });
    if (single && !single.isDir) {
      fileItems.push(
        { label: "Liman'da düzenle", icon: "edit", action: () => (editing = single.path) },
        { label: "Bilgisayardaki editörde aç", icon: "file", action: () => editLocal(single) },
      );
    }
    fileItems.push(
      { label: "İndir…", icon: "download", action: () => download(items) },
      null,
      { label: "Yeniden adlandır", icon: "edit", disabled: !single, action: () => single && startRename(single) },
      { label: "Yolu kopyala", icon: "copy", action: () => writeText(items.map((x) => x.path).join("\n")) },
      null,
      { label: "Sil", icon: "trash", danger: true, action: () => remove(items) },
    );
    menu = {
      x: ev.clientX,
      y: ev.clientY,
      items: e
        ? fileItems
        : [
            { label: "Dosya yükle…", icon: "upload", action: () => uploadFiles() },
            { label: "Klasör yükle…", icon: "upload", action: () => uploadFiles(true) },
            { label: "Yeni klasör", icon: "folderPlus", action: mkdir },
            null,
            { label: "Yenile", icon: "refresh", action: () => load(path) },
            { label: showHidden ? "Gizli dosyaları gizle" : "Gizli dosyaları göster", action: () => (showHidden = !showHidden) },
          ],
    };
  }

  function onKey(ev: KeyboardEvent) {
    if (renaming) return;
    if (ev.key === "Delete" || (ev.key === "Backspace" && ev.metaKey)) remove(selectedEntries);
    else if (ev.key === "Enter" && selectedEntries.length === 1) activate(selectedEntries[0]);
    else if (ev.key === "Backspace") load(parentPath(path));
    else if (ev.key === "F2" && selectedEntries.length === 1) startRename(selectedEntries[0]);
  }
</script>

<div
  class="sftp"
  class:dropping={dropping || fileDrag.drag?.over === "remote"}
  class:wide
  bind:this={panel}
  data-file-drop={localDir !== null ? "remote" : undefined}
>
  <div class="toolbar">
    <button class="icon-btn" title="Üst klasör" onclick={() => load(parentPath(path))}><Icon name="up" /></button>
    <button class="icon-btn" title="Ev klasörü" onclick={async () => load(await api.sftpHome(termId))}><Icon name="home" /></button>
    <button class="icon-btn" title="Yenile" onclick={() => load(path)}><Icon name="refresh" /></button>
    <span class="spacer"></span>
    <button class="icon-btn" title="Dosya yükle" onclick={() => uploadFiles()}><Icon name="upload" /></button>
    <button class="icon-btn" title="Seçilenleri indir" disabled={!selectedEntries.length} onclick={() => download(selectedEntries)}>
      <Icon name="download" />
    </button>
    <button class="icon-btn" title="Yeni klasör" onclick={mkdir}><Icon name="folderPlus" /></button>
    <button class="icon-btn" title="Sil" disabled={!selectedEntries.length} onclick={() => remove(selectedEntries)}>
      <Icon name="trash" />
    </button>
  </div>
  <form
    class="path"
    onsubmit={(e) => {
      e.preventDefault();
      load(pathInput.trim() || "/");
    }}
  >
    <input bind:value={pathInput} spellcheck="false" aria-label="Uzak klasör" />
  </form>
  <label class="hidden-toggle"><input type="checkbox" bind:checked={showHidden} /> Gizli dosyaları göster</label>

  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="list" tabindex="0" onkeydown={onKey} oncontextmenu={(e) => context(e, null)} role="listbox" aria-label="Dosyalar">
    {#if wide}
      <div class="row head" aria-hidden="true">
        <span class="ic"></span>
        <span class="name">Ad</span>
        <span class="size">Boyut</span>
        <span class="date">Değiştirilme</span>
        <span class="perms">İzinler</span>
      </div>
    {/if}
    {#if error}
      <p class="err">{error}</p>
    {/if}
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
          if (localDir === null || renaming) return;
          if (!selected.has(e.path) && !ev.metaKey && !ev.ctrlKey && !ev.shiftKey) selected = new Set([e.path]);
          fileDrag.start(ev, "remote", () => selectedEntries);
        }}
        ondblclick={() => activate(e)}
        oncontextmenu={(ev) => {
          ev.stopPropagation();
          context(ev, e);
        }}
        title="{e.name}\n{e.perms}  {e.isDir ? '' : formatSize(e.size)}  {formatDate(e.mtime)}"
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
        {#if wide}
          <span class="date">{formatDate(e.mtime)}</span>
          <span class="perms">{e.isDir ? "d" : e.isLink ? "l" : "-"}{e.perms}</span>
        {/if}
      </div>
    {:else}
      {#if !loading && !error}<p class="empty">Klasör boş</p>{/if}
    {/each}
  </div>
  {#if dropping || fileDrag.drag?.over === "remote"}
    <div class="drop-hint"><Icon name="upload" size={22} /> {path} klasörüne yükle</div>
  {/if}
</div>

{#if menu}
  <ContextMenu {...menu} onClose={() => (menu = null)} />
{/if}
{#if editing}
  <EditorDialog {termId} path={editing} onClose={() => (editing = null)} />
{/if}

<style>
  .sftp {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 8px 4px;
  }
  .spacer {
    flex: 1;
  }
  .path {
    padding: 2px 8px 4px;
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
    padding: 0 10px 6px;
    font-size: 11.5px;
    color: var(--muted);
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 2px 4px 8px;
    outline: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 3px 6px;
    border-radius: 4px;
    font-size: 12.5px;
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
  .ic {
    display: inline-flex;
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
  .size {
    color: var(--muted);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .date,
  .perms {
    color: var(--muted);
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
  }
  .wide .size {
    width: 80px;
    text-align: right;
    font-size: 11.5px;
  }
  .wide .date {
    width: 130px;
    padding-left: 16px;
  }
  .wide .perms {
    width: 100px;
    font-family: "JetBrains Mono", Menlo, Consolas, monospace;
    font-size: 11px;
  }
  .wide .row {
    padding: 4px 10px;
    font-size: 13px;
  }
  .row.head {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--panel);
    color: var(--muted);
    font-size: 11px !important;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    border-bottom: 1px solid var(--border);
    border-radius: 0;
  }
  .row.head:hover {
    background: var(--panel);
  }
  .row.head .ic {
    width: 15px;
  }
  .wide .toolbar,
  .wide .path,
  .wide .hidden-toggle {
    padding-left: 14px;
    padding-right: 14px;
  }
  .wide .list {
    padding: 0 8px 12px;
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
  .dropping .list {
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
