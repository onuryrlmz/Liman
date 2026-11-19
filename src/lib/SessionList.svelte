<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import BackupDialog from "./BackupDialog.svelte";
  import { api, errText, kindInfo, type Session } from "./api";
  import { store } from "./tabs.svelte";
  import { buildTree, inGroup, leafName, parentOf, type GroupNode } from "./groups";

  let { onEdit }: { onEdit: (s: Session | null, folder?: string | null) => void } = $props();

  let filter = $state("");
  let collapsed = $state<Record<string, boolean>>(loadCollapsed());
  let menu = $state<{ x: number; y: number; items: (MenuItem | null)[] } | null>(null);
  let backup = $state<{ mode: "export" | "import"; group?: string | null } | null>(null);
  /** Satır içi ad düzenleme: yeni grup (`group: null`, `parent` altında) ya da var olan grubun son adı. */
  let naming = $state<{ group: string | null; parent?: string | null; value: string } | null>(null);

  function loadCollapsed(): Record<string, boolean> {
    try {
      return JSON.parse(localStorage.getItem("collapsedGroups") ?? "{}");
    } catch {
      return {};
    }
  }

  function toggle(g: string) {
    collapsed[g] = !collapsed[g];
    try {
      localStorage.setItem("collapsedGroups", JSON.stringify(collapsed));
    } catch {}
  }

  const q = $derived(filter.trim().toLowerCase());
  const matches = (s: Session) => !q || `${s.name} ${s.host} ${s.username} ${s.folder ?? ""}`.toLowerCase().includes(q);
  const byName = (a: Session, b: Session) => a.name.localeCompare(b.name, "tr");

  const tree = $derived(buildTree(store.groups));
  const inSub = (s: Session, g: string) => !!s.folder && inGroup(s.folder, g);
  const directItems = (g: string) => store.sessions.filter((s) => s.folder === g && matches(s)).sort(byName);
  const subtree = (g: string) => store.sessions.filter((s) => inSub(s, g));
  /** Aramada: grubun adı ya da alt ağacındaki bir oturum eşleşiyorsa göster. */
  const shown = (n: GroupNode): boolean =>
    !q || n.path.toLowerCase().includes(q) || subtree(n.path).some(matches) || n.children.some(shown);
  const indent = (depth: number) => `${6 + depth * 14}px`;
  const ungrouped = $derived(
    store.sessions.filter((s) => (!s.folder || !store.groups.includes(s.folder)) && matches(s)).sort(byName),
  );

  const openCount = (s: Session) => store.panes.filter((t) => t.sessionId === s.id && t.status === "open").length;

  async function run(fn: () => Promise<unknown>) {
    try {
      await fn();
      await store.loadSessions();
    } catch (e) {
      store.notify(errText(e), "error");
    }
  }

  // ---------- Oturum işlemleri ----------

  async function remove(s: Session) {
    if (!(await ask(`"${s.name}" oturumu silinsin mi?`, { kind: "warning", title: "Oturumu sil" }))) return;
    run(() => api.sessionDelete(s.id));
  }

  function duplicate(s: Session) {
    run(() => api.sessionSave({ ...s, id: "", name: `${s.name} (kopya)`, hasSecret: false }, null));
  }

  function move(s: Session, folder: string | null) {
    if ((s.folder ?? null) === folder) return;
    run(() => api.sessionMove(s.id, folder));
  }

  /** Grubu `parent`'ın altına (null: en üste) taşır; kendi alt ağacına taşınamaz. */
  function moveGroup(g: string, parent: string | null) {
    if (parent !== null && inGroup(parent, g)) return;
    const full = parent ? `${parent}/${leafName(g)}` : leafName(g);
    if (full !== g) run(() => api.groupRename(g, full));
  }

  /** Grubu başka bir grubun altına (ya da en üste) taşır. */
  function moveGroupMenu(x: number, y: number, g: string) {
    const to = (parent: string | null) => moveGroup(g, parent);
    const items: (MenuItem | null)[] = store.groups
      .filter((x) => !inGroup(x, g))
      .map((x) => ({ label: x, icon: "folder", disabled: parentOf(g) === x, action: () => to(x) }));
    if (items.length) items.push(null);
    items.push({ label: "En üst düzey", icon: "up", disabled: !parentOf(g), action: () => to(null) });
    menu = { x, y, items };
  }

  function moveMenu(x: number, y: number, s: Session) {
    const items: (MenuItem | null)[] = store.groups.map((g) => ({
      label: g,
      icon: "folder",
      disabled: s.folder === g,
      action: () => move(s, g),
    }));
    if (items.length) items.push(null);
    items.push({ label: "Grupsuz", icon: "server", disabled: !s.folder, action: () => move(s, null) });
    menu = { x, y, items };
  }

  function sessionMenu(ev: MouseEvent, s: Session) {
    ev.preventDefault();
    const { clientX: x, clientY: y } = ev;
    menu = {
      x,
      y,
      items: [
        { label: "Bağlan", icon: "bolt", action: () => store.openSession(s) },
        s.kind === "sftp"
          ? { label: "Terminal ile aç (SSH)", icon: "terminal", action: () => store.openSession(s, false) }
          : s.kind === "ssh" || !s.kind
            ? { label: "Yalnızca SFTP aç", icon: "folder", action: () => store.openSession(s, true) }
            : null,
        { label: "Düzenle", icon: "edit", action: () => onEdit(s) },
        { label: "Çoğalt", icon: "copy", action: () => duplicate(s) },
        { label: "Gruba taşı…", icon: "folder", action: () => setTimeout(() => moveMenu(x, y, s)) },
        null,
        { label: "Sil", icon: "trash", danger: true, action: () => remove(s) },
      ],
    };
  }

  // ---------- Grup işlemleri ----------

  function startNewGroup() {
    filter = "";
    naming = { group: null, parent: null, value: "" };
  }

  function startSubGroup(parent: string) {
    filter = "";
    if (collapsed[parent]) toggle(parent);
    naming = { group: null, parent, value: "" };
  }

  async function commitName() {
    if (!naming) return;
    const { group, value } = naming;
    const parent = group === null ? (naming.parent ?? null) : parentOf(group);
    naming = null;
    const name = value.trim();
    if (!name) return;
    const full = parent ? `${parent}/${name}` : name;
    if (full === group) return;
    if (group === null) run(() => api.groupCreate(full));
    else {
      if (collapsed[group]) collapsed[full] = true;
      run(() => api.groupRename(group, full));
    }
  }

  async function deleteGroup(g: string, withSessions: boolean) {
    const n = subtree(g).length;
    if (withSessions && n) {
      const ok = await ask(`"${g}" grubu ve içindeki ${n} oturum kalıcı olarak silinsin mi?`, {
        kind: "warning",
        title: "Grubu sil",
      });
      if (!ok) return;
    }
    run(() => api.groupDelete(g, withSessions));
  }

  function startRename(g: string) {
    naming = { group: g, value: g.slice((parentOf(g)?.length ?? -1) + 1) };
  }

  function groupMenu(ev: MouseEvent, g: string) {
    ev.preventDefault();
    const total = subtree(g).length;
    const { clientX: x, clientY: y } = ev;
    menu = {
      x,
      y,
      items: [
        {
          label: `Tümüne bağlan (${total})`,
          icon: "bolt",
          disabled: !total,
          action: () => subtree(g).forEach((s) => store.openSession(s)),
        },
        { label: "Bu gruba oturum ekle", icon: "plus", action: () => onEdit(null, g) },
        { label: "Alt grup ekle", icon: "folderPlus", action: () => startSubGroup(g) },
        { label: "Yeniden adlandır", icon: "edit", action: () => startRename(g) },
        { label: "Taşı…", icon: "folder", action: () => setTimeout(() => moveGroupMenu(x, y, g)) },
        { label: "Dışarı aktar…", icon: "upload", disabled: !total, action: () => (backup = { mode: "export", group: g }) },
        null,
        {
          label: parentOf(g) ? "Grubu kaldır (içindekiler üst gruba çıkar)" : "Grubu kaldır (oturumlar kalır)",
          icon: "x",
          action: () => deleteGroup(g, false),
        },
        { label: "Grubu oturumlarıyla sil", icon: "trash", danger: true, disabled: !total, action: () => deleteGroup(g, true) },
      ],
    };
  }

  // ---------- Sürükle-bırak ----------
  // Tarayıcının HTML5 sürükle-bırakı Windows'ta Tauri'nin dosya bırakma özelliğiyle çakıştığı
  // için işaretçi olaylarıyla yapıyoruz.

  // Sürüklenen şey bir oturum ya da bir grup (alt gruplarıyla birlikte taşınır).
  type Dragged = { s: Session; g?: undefined } | { g: string; s?: undefined };

  let press: { item: Dragged; x: number; y: number } | null = null;
  let drag = $state<{ item: Dragged; x: number; y: number; target: string | null } | null>(null);
  let justDragged = false;

  const dragLabel = (d: Dragged) => (d.s ? d.s.name : leafName(d.g));

  /** Bırakılabilecek hedef mi? Grup kendi içine ya da zaten bulunduğu yere bırakılmaz. */
  function validTarget(d: Dragged, target: string): boolean {
    if (d.s) return true;
    if (target === "") return parentOf(d.g) !== null;
    return !inGroup(target, d.g) && parentOf(d.g) !== target;
  }

  function onPointerDown(ev: PointerEvent, item: Dragged) {
    if (ev.button !== 0) return;
    press = { item, x: ev.clientX, y: ev.clientY };
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp, { once: true });
  }

  function onPointerMove(ev: PointerEvent) {
    if (!press) return;
    if (!drag && Math.hypot(ev.clientX - press.x, ev.clientY - press.y) < 6) return;
    const el = document.elementFromPoint(ev.clientX, ev.clientY)?.closest<HTMLElement>("[data-drop]");
    const t = el?.dataset.drop ?? null;
    drag = { item: press.item, x: ev.clientX, y: ev.clientY, target: t !== null && validTarget(press.item, t) ? t : null };
  }

  function onPointerUp() {
    window.removeEventListener("pointermove", onPointerMove);
    if (drag) {
      const to = drag.target === "" ? null : drag.target;
      if (drag.target !== null) drag.item.s ? move(drag.item.s, to) : moveGroup(drag.item.g!, to);
      justDragged = true;
      setTimeout(() => (justDragged = false));
    }
    press = null;
    drag = null;
  }

  function open(s: Session) {
    if (!justDragged) store.openSession(s);
  }
</script>

{#snippet sessionRow(s: Session, depth: number)}
  <button
    class="item"
    style:padding-left={depth ? `${10 + depth * 14}px` : undefined}
    class:dragging={drag?.item.s?.id === s.id}
    ondblclick={() => open(s)}
    onkeydown={(e) => e.key === "Enter" && store.openSession(s)}
    oncontextmenu={(e) => sessionMenu(e, s)}
    onpointerdown={(e) => onPointerDown(e, { s })}
    title="{s.username ? s.username + '@' : ''}{s.host}:{s.port}  (çift tıkla: bağlan, sürükle: gruba taşı)"
  >
    <span class="ic"><Icon name={kindInfo[s.kind ?? "ssh"].icon} size={14} /></span>
    <span class="text">
      <span class="name">{s.name}</span>
      <span class="host">
        {#if s.kind === "serial"}{s.host} · {s.baud ?? 115200}{:else}{s.username ? s.username + "@" : ""}{s.host}{s.port !== (s.kind === "telnet" ? 23 : 22) ? ":" + s.port : ""}{/if}
      </span>
    </span>
    {#if kindInfo[s.kind ?? "ssh"].badge}<span class="badge">{kindInfo[s.kind ?? "ssh"].badge}</span>{/if}
    {#if openCount(s)}<span class="dot" title="Açık oturum"></span>{/if}
    {#if s.hasSecret}<span class="key" title="Parola kayıtlı"><Icon name="key" size={12} /></span>{/if}
  </button>
{/snippet}

{#snippet groupNode(node: GroupNode, depth: number)}
  {#if shown(node)}
    {@const open = !collapsed[node.path] || !!q}
    {@const items = directItems(node.path)}
    <div class="group" class:drop={drag?.target === node.path} data-drop={node.path}>
      {#if naming?.group === node.path}
        <div class="folder editing" style:padding-left={indent(depth)}>
          <Icon name="folder" size={14} />
          {@render nameInput("Grup adı")}
        </div>
      {:else}
        <button
          class="folder"
          class:dragging={drag?.item.g === node.path}
          style:padding-left={indent(depth)}
          onpointerdown={(e) => onPointerDown(e, { g: node.path })}
          onclick={() => !justDragged && toggle(node.path)}
          ondblclick={() => startRename(node.path)}
          oncontextmenu={(e) => groupMenu(e, node.path)}
          title="{node.path}  (sürükle: başka bir grubun içine taşı)"
        >
          <span class="chev" class:open><Icon name="chevron" size={12} /></span>
          <Icon name="folder" size={14} />
          <span class="gname">{node.name}</span>
          <span class="count">{subtree(node.path).length}</span>
        </button>
      {/if}
      {#if open}
        {#if naming && naming.group === null && naming.parent === node.path}
          <div class="folder editing" style:padding-left={indent(depth + 1)}>
            <Icon name="folder" size={14} />
            {@render nameInput("Alt grup adı")}
          </div>
        {/if}
        {#each node.children as child (child.path)}
          {@render groupNode(child, depth + 1)}
        {/each}
        {#each items as s (s.id)}
          {@render sessionRow(s, depth + 1)}
        {/each}
        {#if !items.length && !node.children.length && !q}
          <div class="empty-group" style:padding-left={indent(depth + 1)}>
            {drag ? "Buraya bırakın" : "Boş grup — oturumları sürükleyip bırakın"}
          </div>
        {/if}
      {/if}
    </div>
  {/if}
{/snippet}

{#snippet nameInput(placeholder: string)}
  <!-- svelte-ignore a11y_autofocus -->
  <input
    class="name-input"
    bind:value={naming!.value}
    {placeholder}
    autofocus
    spellcheck="false"
    onkeydown={(e) => {
      if (e.key === "Enter") commitName();
      if (e.key === "Escape") naming = null;
    }}
    onblur={commitName}
  />
{/snippet}

<div class="sessions">
  <div class="top">
    <div class="search">
      <Icon name="search" size={14} />
      <input bind:value={filter} placeholder="Oturum ara" spellcheck="false" />
    </div>
    <div class="actions">
      <button class="icon-btn" title="Yeni oturum" onclick={() => onEdit(null)}><Icon name="plus" size={15} /></button>
      <button class="icon-btn" title="Yeni grup" onclick={startNewGroup}><Icon name="folderPlus" size={15} /></button>
      <span class="spacer"></span>
      <button class="icon-btn" title="İçeri aktar" onclick={() => (backup = { mode: "import" })}><Icon name="download" size={15} /></button>
      <button class="icon-btn" title="Dışarı aktar" disabled={!store.sessions.length} onclick={() => (backup = { mode: "export" })}>
        <Icon name="upload" size={15} />
      </button>
    </div>
  </div>

  <div class="list">
    {#if naming && naming.group === null && !naming.parent}
      <div class="folder editing">
        <Icon name="folder" size={14} />
        {@render nameInput("Grup adı")}
      </div>
    {/if}

    {#each tree as node (node.path)}
      {@render groupNode(node, 0)}
    {/each}

    {#if ungrouped.length || (drag && (drag.item.s ? drag.item.s.folder : parentOf(drag.item.g)))}
      <div class="group loose" class:drop={drag?.target === ""} data-drop="">
        {#if tree.length}<div class="folder plain">{drag?.item.g ? "En üst düzey" : "Grupsuz"}</div>{/if}
        {#each ungrouped as s (s.id)}
          {@render sessionRow(s, 0)}
        {:else}
          <div class="empty-group">{drag?.item.g ? "En üst düzeye taşımak için buraya bırakın" : "Gruptan çıkarmak için buraya bırakın"}</div>
        {/each}
      </div>
    {/if}

    {#if !tree.length && !ungrouped.length && !naming}
      <div class="empty">
        {#if filter}
          Eşleşen oturum yok.
        {:else}
          <p>Henüz kayıtlı oturum yok.</p>
          <button class="btn primary" onclick={() => onEdit(null)}><Icon name="plus" size={14} /> Yeni oturum</button>
          <button class="btn" onclick={() => (backup = { mode: "import" })}><Icon name="download" size={14} /> İçeri aktar</button>
        {/if}
      </div>
    {/if}
  </div>
</div>

{#if drag}
  <div class="ghost" style:left="{drag.x + 12}px" style:top="{drag.y + 8}px">
    <Icon name={drag.item.g ? "folder" : "server"} size={13} />
    {dragLabel(drag.item)}
    {#if drag.target !== null}<span>→ {drag.target || (drag.item.g ? "En üst düzey" : "Grupsuz")}</span>{/if}
  </div>
{/if}
{#if menu}
  <ContextMenu {...menu} onClose={() => (menu = null)} />
{/if}
{#if backup}
  <BackupDialog mode={backup.mode} group={backup.group} onClose={() => (backup = null)} />
{/if}

<style>
  .sessions {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .top {
    padding: 8px 8px 4px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--muted);
  }
  .search input {
    flex: 1;
    border: 0;
    background: none;
    padding: 6px 0;
    outline: none;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-top: 6px;
  }
  .spacer {
    flex: 1;
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 0 6px 10px;
  }
  .group {
    border-radius: 6px;
    border: 1px solid transparent;
  }
  .group.drop {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .folder,
  button.item {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    border: 0;
    background: none;
    color: var(--text);
    text-align: left;
    border-radius: 5px;
    cursor: default;
    font: inherit;
  }
  .folder {
    padding: 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }
  .folder :global(svg) {
    flex: none;
  }
  .folder.plain {
    padding: 8px 6px 4px;
  }
  .folder.editing {
    color: var(--warn);
  }
  .gname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name-input {
    flex: 1;
    min-width: 0;
    padding: 3px 6px;
    font-size: 12.5px;
    font-weight: 400;
  }
  .chev {
    display: inline-flex;
    transition: transform 0.12s;
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .count {
    margin-left: auto;
    font-weight: 400;
    font-size: 11px;
  }
  button.item {
    padding: 5px 8px;
    touch-action: none;
  }
  button.item.dragging,
  button.folder.dragging {
    opacity: 0.4;
  }
  button.item:hover,
  button.folder:hover {
    background: var(--hover);
  }
  button.item:focus-visible {
    outline: 1px solid var(--accent);
  }
  .ic {
    display: inline-flex;
    color: var(--accent);
    flex: none;
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .name,
  .host {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name {
    font-size: 13px;
  }
  .host {
    font-size: 11px;
    color: var(--muted);
  }
  .badge {
    flex: none;
    padding: 1px 5px;
    border-radius: 4px;
    border: 1px solid var(--border);
    color: var(--muted);
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.4px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--ok);
    flex: none;
  }
  .key {
    color: var(--muted);
    display: inline-flex;
  }
  .empty-group {
    padding: 4px 8px 8px 30px;
    font-size: 11.5px;
    color: var(--muted);
    font-style: italic;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 16px 10px;
    font-size: 12.5px;
    color: var(--muted);
    text-align: center;
  }
  .empty p {
    margin: 0 0 4px;
  }
  .ghost {
    position: fixed;
    z-index: 95;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    border-radius: 6px;
    background: var(--panel-2);
    border: 1px solid var(--accent);
    box-shadow: 0 6px 20px rgb(0 0 0 / 0.4);
    font-size: 12.5px;
    pointer-events: none;
    white-space: nowrap;
  }
  .ghost :global(svg) {
    color: var(--accent);
  }
  .ghost span {
    color: var(--accent);
  }
</style>
