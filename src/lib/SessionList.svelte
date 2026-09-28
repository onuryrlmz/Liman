<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import { api, errText, type Session } from "./api";
  import { store } from "./tabs.svelte";

  let { onEdit }: { onEdit: (s: Session | null) => void } = $props();

  let filter = $state("");
  let collapsed = $state<Record<string, boolean>>({});
  let menu = $state<{ x: number; y: number; items: (MenuItem | null)[] } | null>(null);

  const groups = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const list = store.sessions
      .filter((s) => !q || `${s.name} ${s.host} ${s.username} ${s.folder ?? ""}`.toLowerCase().includes(q))
      .sort((a, b) => a.name.localeCompare(b.name, "tr"));
    const map = new Map<string, Session[]>();
    for (const s of list) {
      const f = s.folder || "";
      if (!map.has(f)) map.set(f, []);
      map.get(f)!.push(s);
    }
    return [...map.entries()].sort(([a], [b]) => (a === "" ? 1 : b === "" ? -1 : a.localeCompare(b, "tr")));
  });

  const openCount = (s: Session) => store.tabs.filter((t) => t.sessionId === s.id && t.status === "open").length;

  async function remove(s: Session) {
    if (!(await ask(`"${s.name}" oturumu silinsin mi?`, { kind: "warning", title: "Oturumu sil" }))) return;
    try {
      await api.sessionDelete(s.id);
      await store.loadSessions();
    } catch (e) {
      store.notify(errText(e), "error");
    }
  }

  async function duplicate(s: Session) {
    try {
      await api.sessionSave({ ...s, id: "", name: `${s.name} (kopya)`, hasSecret: false }, null);
      await store.loadSessions();
    } catch (e) {
      store.notify(errText(e), "error");
    }
  }

  function context(ev: MouseEvent, s: Session) {
    ev.preventDefault();
    menu = {
      x: ev.clientX,
      y: ev.clientY,
      items: [
        { label: "Bağlan", icon: "bolt", action: () => store.openSession(s) },
        { label: "Düzenle", icon: "edit", action: () => onEdit(s) },
        { label: "Çoğalt", icon: "copy", action: () => duplicate(s) },
        null,
        { label: "Sil", icon: "trash", danger: true, action: () => remove(s) },
      ],
    };
  }
</script>

<div class="sessions">
  <div class="search">
    <Icon name="search" size={14} />
    <input bind:value={filter} placeholder="Oturum ara" spellcheck="false" />
  </div>
  <div class="list">
    {#each groups as [folder, items] (folder)}
      {#if folder}
        <button class="folder" onclick={() => (collapsed[folder] = !collapsed[folder])}>
          <span class="chev" class:open={!collapsed[folder]}><Icon name="chevron" size={12} /></span>
          <Icon name="folder" size={14} />
          {folder}
          <span class="count">{items.length}</span>
        </button>
      {/if}
      {#if !collapsed[folder]}
        {#each items as s (s.id)}
          <button
            class="item"
            class:nested={!!folder}
            ondblclick={() => store.openSession(s)}
            onkeydown={(e) => e.key === "Enter" && store.openSession(s)}
            oncontextmenu={(e) => context(e, s)}
            title="{s.username ? s.username + '@' : ''}{s.host}:{s.port}  (çift tıkla: bağlan)"
          >
            <span class="ic"><Icon name="server" size={14} /></span>
            <span class="text">
              <span class="name">{s.name}</span>
              <span class="host">{s.username ? s.username + "@" : ""}{s.host}{s.port !== 22 ? ":" + s.port : ""}</span>
            </span>
            {#if openCount(s)}<span class="dot" title="Açık oturum"></span>{/if}
            {#if s.hasSecret}<span class="key" title="Parola kayıtlı"><Icon name="key" size={12} /></span>{/if}
          </button>
        {/each}
      {/if}
    {:else}
      <div class="empty">
        {#if filter}
          Eşleşen oturum yok.
        {:else}
          <p>Henüz kayıtlı oturum yok.</p>
          <button class="btn primary" onclick={() => onEdit(null)}><Icon name="plus" size={14} /> Yeni oturum</button>
        {/if}
      </div>
    {/each}
  </div>
</div>

{#if menu}
  <ContextMenu {...menu} onClose={() => (menu = null)} />
{/if}

<style>
  .sessions {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 8px;
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
  .list {
    flex: 1;
    overflow: auto;
    padding: 0 6px 10px;
  }
  button.folder,
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
  }
  button.folder {
    padding: 6px 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
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
  }
  button.item.nested {
    padding-left: 24px;
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
  .empty {
    padding: 16px 10px;
    font-size: 12.5px;
    color: var(--muted);
    text-align: center;
  }
  .empty p {
    margin: 0 0 10px;
  }
</style>
