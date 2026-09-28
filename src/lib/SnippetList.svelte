<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import type { Snippet } from "./api";
  import { store } from "./tabs.svelte";

  let filter = $state("");
  let editing = $state<Snippet | null>(null);
  let menu = $state<{ x: number; y: number; items: (MenuItem | null)[] } | null>(null);

  const q = $derived(filter.trim().toLowerCase());
  const list = $derived(
    store.snippets
      .filter((s) => !q || `${s.name} ${s.command}`.toLowerCase().includes(q))
      .sort((a, b) => a.name.localeCompare(b.name, "tr")),
  );
  const target = $derived(store.active?.kind !== "sftp" && store.active?.status === "open" ? store.active : null);

  function startNew() {
    editing = { id: "", name: "", command: "", run: true };
  }

  function save() {
    if (!editing) return;
    const s = { ...editing, name: editing.name.trim(), command: editing.command.replace(/\s+$/, "") };
    if (!s.command) return;
    if (!s.name) s.name = s.command.split("\n")[0].slice(0, 40);
    const list = [...store.snippets];
    if (s.id) {
      const i = list.findIndex((x) => x.id === s.id);
      if (i >= 0) list[i] = s;
    } else {
      s.id = crypto.randomUUID();
      list.push(s);
    }
    store.saveSnippets(list);
    editing = null;
  }

  async function remove(s: Snippet) {
    if (!(await ask(`"${s.name}" parçacığı silinsin mi?`, { kind: "warning", title: "Parçacığı sil" }))) return;
    store.saveSnippets(store.snippets.filter((x) => x.id !== s.id));
  }

  function context(ev: MouseEvent, s: Snippet) {
    ev.preventDefault();
    menu = {
      x: ev.clientX,
      y: ev.clientY,
      items: [
        { label: "Gönder", icon: "bolt", disabled: !target, action: () => store.runSnippet(s) },
        { label: "Çalıştırmadan yaz", icon: "edit", disabled: !target, action: () => store.runSnippet({ ...s, run: false }) },
        null,
        { label: "Düzenle", icon: "edit", action: () => (editing = { ...s }) },
        { label: "Çoğalt", icon: "copy", action: () => store.saveSnippets([...store.snippets, { ...s, id: crypto.randomUUID(), name: `${s.name} (kopya)` }]) },
        null,
        { label: "Sil", icon: "trash", danger: true, action: () => remove(s) },
      ],
    };
  }
</script>

<div class="snippets">
  <div class="top">
    <div class="search">
      <Icon name="search" size={14} />
      <input bind:value={filter} placeholder="Parçacık ara" spellcheck="false" />
    </div>
    <button class="icon-btn" title="Yeni parçacık" onclick={startNew}><Icon name="plus" size={15} /></button>
  </div>

  {#if editing}
    <form
      class="editor"
      onsubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={editing.name} placeholder="Ad (örn. Disk kullanımı)" autofocus />
      <textarea
        bind:value={editing.command}
        placeholder="Komut (birden fazla satır olabilir)"
        rows="4"
        spellcheck="false"
        onkeydown={(e) => {
          if ((e.metaKey || e.ctrlKey) && e.key === "Enter") save();
          if (e.key === "Escape") editing = null;
        }}
      ></textarea>
      <label class="check"><input type="checkbox" bind:checked={editing.run} /> Gönderince çalıştır (Enter)</label>
      <div class="row">
        <button type="button" class="btn" onclick={() => (editing = null)}>Vazgeç</button>
        <button type="submit" class="btn primary" disabled={!editing.command.trim()}>Kaydet</button>
      </div>
    </form>
  {/if}

  <div class="list">
    {#each list as s (s.id)}
      <button
        class="item"
        onclick={() => store.runSnippet(s)}
        oncontextmenu={(e) => context(e, s)}
        disabled={!target}
        title={target ? `${target.title} terminaline gönder${s.run ? " ve çalıştır" : ""}\n\n${s.command}` : "Bağlı bir terminal seçin"}
      >
        <span class="ic"><Icon name={s.run ? "bolt" : "edit"} size={13} /></span>
        <span class="text">
          <span class="name">{s.name}</span>
          <code>{s.command.split("\n")[0]}{s.command.includes("\n") ? " …" : ""}</code>
        </span>
      </button>
    {:else}
      {#if !editing}
        <div class="empty">
          {#if filter}
            Eşleşen parçacık yok.
          {:else}
            <p>Sık kullandığın komutları kaydet, tek tıkla terminale gönder. MultiExec açıksa tüm panolara gider.</p>
            <button class="btn primary" onclick={startNew}><Icon name="plus" size={14} /> Yeni parçacık</button>
          {/if}
        </div>
      {/if}
    {/each}
  </div>
</div>

{#if menu}
  <ContextMenu {...menu} onClose={() => (menu = null)} />
{/if}

<style>
  .snippets {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px;
  }
  .search {
    flex: 1;
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
    min-width: 0;
    border: 0;
    background: none;
    padding: 6px 0;
    outline: none;
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0 8px 8px;
    padding: 8px;
    border: 1px solid var(--accent);
    border-radius: 8px;
    background: var(--bg);
  }
  .editor input,
  .editor textarea {
    background: var(--panel);
    font-size: 12.5px;
  }
  .editor textarea {
    resize: vertical;
    font-family: "JetBrains Mono", Menlo, Consolas, monospace;
    font-size: 12px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--muted);
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .row .btn {
    padding: 5px 10px;
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 0 6px 10px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .item:hover:not(:disabled) {
    background: var(--hover);
  }
  .item:disabled {
    cursor: default;
    opacity: 0.55;
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
  }
  .name {
    font-size: 13px;
  }
  .name,
  code {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  code {
    font-size: 11px;
    color: var(--muted);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 16px 10px;
    font-size: 12.5px;
    color: var(--muted);
    text-align: center;
  }
  .empty p {
    margin: 0;
    line-height: 1.5;
  }
</style>
