<script lang="ts" module>
  export interface PaletteItem {
    label: string;
    detail?: string;
    icon: string;
    kind: string;
    action: () => void;
  }

  /** Harflerin sırayla geçtiği eşleşme; ardışık ve kelime başı eşleşmeler öne çıkar. */
  export function fuzzyScore(query: string, text: string): number {
    if (!query) return 1;
    const q = query.toLocaleLowerCase("tr");
    const t = text.toLocaleLowerCase("tr");
    let score = 0;
    let ti = 0;
    let streak = 0;
    for (const ch of q) {
      const found = t.indexOf(ch, ti);
      if (found < 0) return 0;
      streak = found === ti ? streak + 1 : 0;
      score += 1 + streak * 2 + (found === 0 || /[\s\-_/.@]/.test(t[found - 1]) ? 3 : 0);
      ti = found + 1;
    }
    return score - t.length * 0.01;
  }
</script>

<script lang="ts">
  import { tick } from "svelte";
  import Icon from "./Icon.svelte";

  let { items, onClose }: { items: PaletteItem[]; onClose: () => void } = $props();

  let query = $state("");
  let index = $state(0);
  let listEl = $state<HTMLDivElement>();

  const results = $derived(
    items
      .map((it) => ({ it, score: Math.max(fuzzyScore(query, it.label), fuzzyScore(query, `${it.kind} ${it.label} ${it.detail ?? ""}`) * 0.8) }))
      .filter((r) => r.score > 0)
      .sort((a, b) => b.score - a.score)
      .slice(0, 60)
      .map((r) => r.it),
  );

  $effect(() => {
    query;
    index = 0;
  });

  async function move(d: number) {
    if (!results.length) return;
    index = (index + d + results.length) % results.length;
    await tick();
    listEl?.querySelector(".on")?.scrollIntoView({ block: "nearest" });
  }

  function run(it: PaletteItem | undefined) {
    if (!it) return;
    onClose();
    it.action();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="overlay" onmousedown={(e) => e.target === e.currentTarget && onClose()}>
  <div class="palette" role="dialog" aria-label="Komut paleti">
    <div class="input">
      <Icon name="search" size={15} />
      <!-- svelte-ignore a11y_autofocus -->
      <input
        bind:value={query}
        placeholder="Oturum, parçacık ya da komut ara…"
        spellcheck="false"
        autofocus
        onkeydown={(e) => {
          if (e.key === "ArrowDown") (e.preventDefault(), move(1));
          else if (e.key === "ArrowUp") (e.preventDefault(), move(-1));
          else if (e.key === "Enter") (e.preventDefault(), run(results[index]));
          else if (e.key === "Escape") (e.preventDefault(), onClose());
        }}
      />
    </div>
    <div class="list" bind:this={listEl} role="listbox">
      {#each results as it, i}
        <button class="item" class:on={i === index} role="option" aria-selected={i === index} onmousemove={() => (index = i)} onclick={() => run(it)}>
          <span class="ic"><Icon name={it.icon} size={14} /></span>
          <span class="label">{it.label}</span>
          {#if it.detail}<span class="detail">{it.detail}</span>{/if}
          <span class="kind">{it.kind}</span>
        </button>
      {:else}
        <p class="empty">Sonuç yok.</p>
      {/each}
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 85;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
    background: rgb(5 8 12 / 0.45);
  }
  .palette {
    width: min(600px, calc(100vw - 32px));
    max-height: 60vh;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: 0 24px 70px rgb(0 0 0 / 0.55);
    overflow: hidden;
  }
  .input {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 14px;
    border-bottom: 1px solid var(--border);
    color: var(--muted);
  }
  .input input {
    flex: 1;
    border: 0;
    background: none;
    padding: 12px 0;
    font-size: 15px;
  }
  .list {
    overflow: auto;
    padding: 6px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border: 0;
    border-radius: 7px;
    background: none;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .item.on {
    background: var(--accent-soft);
  }
  .ic {
    display: inline-flex;
    color: var(--accent);
  }
  .label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .detail {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
    font-size: 12px;
  }
  .kind {
    margin-left: auto;
    flex: none;
    font-size: 11px;
    color: var(--muted);
  }
  .empty {
    margin: 0;
    padding: 14px;
    color: var(--muted);
    font-size: 13px;
  }
</style>
