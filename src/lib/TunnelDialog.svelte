<script lang="ts" module>
  import type { TunnelKind, TunnelSpec } from "./api";

  export const tunnelKinds: Record<TunnelKind, { label: string; flag: string; hint: string }> = {
    local: {
      label: "Yerel",
      flag: "-L",
      hint: "Bu bilgisayardaki bir portu, sunucunun ulaşabildiği bir adrese bağlar. Örnek: 5433 → localhost:5432 ile sunucudaki PostgreSQL'e 127.0.0.1:5433 üzerinden bağlanırsınız.",
    },
    remote: {
      label: "Uzak",
      flag: "-R",
      hint: "Sunucuda bir port açar ve gelen bağlantıları bu bilgisayarın ulaşabildiği bir adrese iletir. Örnek: sunucuda 8080 → 127.0.0.1:3000 ile yerelde çalışan uygulamanızı sunucudan erişilebilir yaparsınız.",
    },
    socks: {
      label: "SOCKS5",
      flag: "-D",
      hint: "Bu bilgisayarda bir SOCKS5 vekil sunucu açar; tarayıcınızı ya da başka bir uygulamayı buna yönlendirince trafik sunucu üzerinden çıkar.",
    },
  };

  export function describeTunnel(t: TunnelSpec) {
    if (t.kind === "socks") return `SOCKS5 127.0.0.1:${t.listenPort}`;
    if (t.kind === "local") return `127.0.0.1:${t.listenPort} → ${t.targetHost}:${t.targetPort}`;
    return `sunucu ${t.listenHost}:${t.listenPort} → ${t.targetHost}:${t.targetPort}`;
  }
</script>

<script lang="ts">
  import Modal from "./Modal.svelte";
  import Icon from "./Icon.svelte";
  import { api, errText, type TunnelInfo } from "./api";
  import { store, type Tab } from "./tabs.svelte";

  let { tab, onClose }: { tab: Tab; onClose: () => void } = $props();

  let tunnels = $state<TunnelInfo[]>([]);
  let kind = $state<TunnelKind>("local");
  let listenHost = $state("localhost");
  let listenPort = $state<number | null>(null);
  let targetHost = $state("localhost");
  let targetPort = $state<number | null>(null);
  let save = $state(false);
  let error = $state("");
  let busy = $state(false);

  const session = $derived(tab.sessionId ? store.sessions.find((s) => s.id === tab.sessionId) : undefined);
  const saved = $derived<TunnelSpec[]>(session?.tunnels ?? []);

  $effect(() => {
    // Tür değişince hedef için mantıklı varsayılan.
    targetHost = kind === "remote" ? "127.0.0.1" : "localhost";
  });

  async function refresh() {
    if (!tab.termId) return;
    try {
      tunnels = await api.tunnelList(tab.termId);
    } catch (e) {
      error = errText(e);
    }
  }
  refresh();

  const sameSpec = (a: TunnelSpec, b: TunnelSpec) =>
    a.kind === b.kind && a.listenPort === b.listenPort && a.targetHost === b.targetHost && a.targetPort === b.targetPort;

  async function persist(list: TunnelSpec[]) {
    if (!session) return;
    await api.sessionSave({ ...session, tunnels: list }, null);
    await store.loadSessions();
  }

  async function add(e: Event) {
    e.preventDefault();
    error = "";
    if (!tab.termId) return;
    if (kind !== "socks" && !targetPort) {
      error = "Hedef port gerekli.";
      return;
    }
    if (kind === "socks" && !listenPort) {
      error = "SOCKS için bir port seçin (örn. 1080).";
      return;
    }
    const spec: TunnelSpec = {
      kind,
      listenHost: kind === "remote" ? listenHost.trim() || "localhost" : "127.0.0.1",
      listenPort: listenPort ?? (kind === "local" ? targetPort! : 0),
      targetHost: kind === "socks" ? "" : targetHost.trim() || "localhost",
      targetPort: kind === "socks" ? 0 : targetPort!,
    };
    busy = true;
    try {
      await api.tunnelStart(tab.termId, spec);
      if (save && !saved.some((s) => sameSpec(s, spec))) await persist([...saved, spec]);
      listenPort = null;
      targetPort = null;
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

<Modal title="Tüneller — {tab.title}" width={560} {onClose}>
  <div class="kinds" role="radiogroup" aria-label="Tünel türü">
    {#each Object.entries(tunnelKinds) as [k, v]}
      <button role="radio" aria-checked={kind === k} class:on={kind === k} onclick={() => (kind = k as TunnelKind)}>
        {v.label} <span>{v.flag}</span>
      </button>
    {/each}
  </div>
  <p class="hint">{tunnelKinds[kind].hint}</p>

  {#if !tab.termId}
    <p class="err">Tünel açmak için oturumun bağlı olması gerekir.</p>
  {:else}
    <form class="add" onsubmit={add}>
      {#if kind === "remote"}
        <label class="grow">
          <span>Sunucuda dinlenecek adres</span>
          <input bind:value={listenHost} spellcheck="false" placeholder="localhost" />
        </label>
      {/if}
      <label>
        <span>{kind === "remote" ? "Sunucu portu" : "Yerel port"}</span>
        <input
          type="number"
          min="0"
          max="65535"
          bind:value={listenPort}
          placeholder={kind === "socks" ? "1080" : kind === "local" && targetPort ? String(targetPort) : "otomatik"}
        />
      </label>
      {#if kind !== "socks"}
        <span class="arrow">→</span>
        <label class="grow">
          <span>{kind === "remote" ? "Bu bilgisayardan hedef" : "Sunucudan hedef"}</span>
          <input bind:value={targetHost} spellcheck="false" />
        </label>
        <label>
          <span>Hedef port</span>
          <input type="number" min="1" max="65535" bind:value={targetPort} placeholder="örn. 80" />
        </label>
      {/if}
      <button class="btn primary" type="submit" disabled={busy}>Başlat</button>
    </form>
    {#if session}
      <label class="check"><input type="checkbox" bind:checked={save} /> Bu oturuma kaydet — bağlanınca otomatik başlasın</label>
    {/if}
    {#if error}<p class="err">{error}</p>{/if}

    <h4>Açık tüneller</h4>
    <div class="list">
      {#each tunnels as t (t.id)}
        <div class="item">
          <span class="badge">{tunnelKinds[t.kind].flag}</span>
          <code>{describeTunnel(t)}</code>
          <button class="icon-btn" title="Durdur" onclick={() => stop(t)}><Icon name="x" /></button>
        </div>
      {:else}
        <p class="empty">Bu bağlantıda açık tünel yok.</p>
      {/each}
    </div>

    {#if saved.length}
      <h4>Oturuma kayıtlı (bağlanınca başlar)</h4>
      <div class="list">
        {#each saved as t, i (i)}
          <div class="item saved">
            <span class="badge">{tunnelKinds[t.kind].flag}</span>
            <code>{describeTunnel(t)}</code>
            <button class="icon-btn" title="Kayıttan kaldır" onclick={() => persist(saved.filter((_, j) => j !== i))}><Icon name="trash" /></button>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</Modal>

<style>
  .kinds {
    display: flex;
    gap: 6px;
    margin-bottom: 10px;
  }
  .kinds button {
    padding: 5px 12px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 12.5px;
    cursor: pointer;
  }
  .kinds button span {
    font-family: "JetBrains Mono", Menlo, monospace;
    font-size: 11px;
    opacity: 0.7;
  }
  .kinds button.on {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
  }
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
    flex-wrap: wrap;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: var(--muted);
    width: 96px;
  }
  label.grow {
    flex: 1;
    min-width: 140px;
    width: auto;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    width: auto;
    margin-top: 10px;
    color: var(--text);
    font-size: 12.5px;
  }
  .arrow {
    color: var(--muted);
    padding-bottom: 7px;
  }
  h4 {
    margin: 18px 0 6px;
    font-size: 11.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--muted);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: 7px;
  }
  .item.saved {
    border-style: dashed;
  }
  .item code {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    flex: none;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--accent-soft);
    color: var(--accent);
    font: 600 11px "JetBrains Mono", Menlo, monospace;
  }
  .empty {
    margin: 0;
    color: var(--muted);
    font-size: 13px;
  }
  .err {
    color: var(--danger);
    font-size: 13px;
  }
</style>
