<script lang="ts">
  import Modal from "./Modal.svelte";
  import Icon from "./Icon.svelte";
  import { api, errText, type TunnelInfo } from "./api";
  import type { Tab } from "./tabs.svelte";

  let { tab, onClose }: { tab: Tab; onClose: () => void } = $props();

  let tunnels = $state<TunnelInfo[]>([]);
  let localPort = $state<number | null>(null);
  let remoteHost = $state("localhost");
  let remotePort = $state<number | null>(null);
  let error = $state("");
  let busy = $state(false);

  async function refresh() {
    if (!tab.termId) return;
    try {
      tunnels = await api.tunnelList(tab.termId);
    } catch (e) {
      error = errText(e);
    }
  }
  refresh();

  async function add(e: Event) {
    e.preventDefault();
    error = "";
    if (!tab.termId) return;
    if (!remotePort) {
      error = "Uzak port gerekli.";
      return;
    }
    busy = true;
    try {
      await api.tunnelStart(tab.termId, localPort ?? remotePort, remoteHost.trim() || "localhost", remotePort);
      localPort = null;
      remotePort = null;
      await refresh();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function stop(t: TunnelInfo) {
    if (!tab.termId) return;
    await api.tunnelStop(tab.termId, t.id);
    await refresh();
  }
</script>

<Modal title="Tüneller — {tab.title}" width={520} {onClose}>
  <p class="hint">
    Yerel bir portu SSH üzerinden uzak bir adrese yönlendirir. Örnek: yerel 5433 → <code>localhost:5432</code> ile
    sunucudaki PostgreSQL'e <code>127.0.0.1:5433</code> üzerinden bağlanırsınız.
  </p>

  {#if !tab.termId}
    <p class="err">Tünel açmak için oturumun bağlı olması gerekir.</p>
  {:else}
    <form class="add" onsubmit={add}>
      <label>
        <span>Yerel port</span>
        <input type="number" min="0" max="65535" bind:value={localPort} placeholder={remotePort ? String(remotePort) : "örn. 8080"} />
      </label>
      <span class="arrow">→</span>
      <label class="grow">
        <span>Uzak adres (sunucudan bakınca)</span>
        <input bind:value={remoteHost} spellcheck="false" />
      </label>
      <label>
        <span>Uzak port</span>
        <input type="number" min="1" max="65535" bind:value={remotePort} placeholder="örn. 80" />
      </label>
      <button class="btn primary" type="submit" disabled={busy}>Başlat</button>
    </form>
    {#if error}<p class="err">{error}</p>{/if}

    <div class="list">
      {#each tunnels as t (t.id)}
        <div class="item">
          <Icon name="tunnel" />
          <code>127.0.0.1:{t.localPort}</code>
          <span class="arrow">→</span>
          <code>{t.remoteHost}:{t.remotePort}</code>
          <button class="icon-btn" title="Durdur" onclick={() => stop(t)}><Icon name="x" /></button>
        </div>
      {:else}
        <p class="empty">Bu oturumda açık tünel yok.</p>
      {/each}
    </div>
  {/if}
</Modal>

<style>
  .hint {
    margin: 0 0 14px;
    color: var(--muted);
    font-size: 12.5px;
    line-height: 1.5;
  }
  .add {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: var(--muted);
    width: 90px;
  }
  label.grow {
    flex: 1;
    width: auto;
  }
  .arrow {
    color: var(--muted);
    padding-bottom: 7px;
  }
  .list {
    margin-top: 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--accent);
  }
  .item code {
    color: var(--text);
  }
  .item button {
    margin-left: auto;
  }
  .empty {
    color: var(--muted);
    font-size: 13px;
  }
  .err {
    color: var(--danger);
    font-size: 13px;
  }
</style>
