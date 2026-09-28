<script lang="ts">
  import Modal from "./Modal.svelte";
  import Icon from "./Icon.svelte";
  import { api, errText, formatDate, formatSize } from "./api";
  import { store } from "./tabs.svelte";

  let { termId, local, remote, onClose }: { termId: string; local: string; remote: string; onClose: () => void } = $props();

  type Status = "onlyLocal" | "onlyRemote" | "localNewer" | "remoteNewer" | "different";
  interface Diff {
    path: string;
    status: Status;
    localSize: number | null;
    remoteSize: number | null;
    localMtime: number | null;
    remoteMtime: number | null;
  }
  type Choice = "upload" | "download" | "skip";

  const labels: Record<Status, string> = {
    onlyLocal: "Yalnızca yerelde",
    onlyRemote: "Yalnızca sunucuda",
    localNewer: "Yereldeki daha yeni",
    remoteNewer: "Sunucudaki daha yeni",
    different: "Boyut farklı",
  };
  const defaultChoice = (s: Status): Choice =>
    s === "onlyLocal" || s === "localNewer" ? "upload" : s === "onlyRemote" || s === "remoteNewer" ? "download" : "skip";

  let loading = $state(true);
  let error = $state("");
  let diffs = $state<Diff[]>([]);
  let same = $state(0);
  let truncated = $state(false);
  let choices = $state<Record<string, Choice>>({});
  let busy = $state(false);

  async function compare() {
    loading = true;
    error = "";
    try {
      const c = await api.syncCompare(termId, local, remote);
      diffs = c.diffs as Diff[];
      same = c.same;
      truncated = c.truncated;
      choices = Object.fromEntries(diffs.map((d) => [d.path, defaultChoice(d.status)]));
    } catch (e) {
      error = errText(e);
    } finally {
      loading = false;
    }
  }
  compare();

  const planned = $derived(diffs.filter((d) => choices[d.path] !== "skip"));
  const ups = $derived(planned.filter((d) => choices[d.path] === "upload").length);
  const downs = $derived(planned.length - ups);

  function setAll(fn: (d: Diff) => Choice) {
    choices = Object.fromEntries(diffs.map((d) => [d.path, fn(d)]));
  }

  async function apply() {
    busy = true;
    try {
      const failed = await api.syncApply(
        termId,
        local,
        remote,
        planned.map((d) => ({ path: d.path, direction: choices[d.path] as "upload" | "download" })),
      );
      store.notify(failed ? `${failed} dosya aktarılamadı` : `${planned.length} dosya eşitlendi`, failed ? "error" : "info");
      onClose();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  const can = (d: Diff, c: Choice) =>
    c === "skip" || (c === "upload" ? d.status !== "onlyRemote" : d.status !== "onlyLocal");
</script>

<Modal title="Klasörleri eşitle" width={760} {onClose}>
  <div class="roots">
    <div><span>Bu bilgisayar</span><code title={local}>{local}</code></div>
    <Icon name="chevron" size={14} />
    <div><span>Sunucu</span><code title={remote}>{remote}</code></div>
  </div>

  {#if loading}
    <p class="muted">Karşılaştırılıyor… (alt klasörler dahil)</p>
  {:else if error}
    <p class="err">{error}</p>
  {:else if !diffs.length}
    <p class="ok">Klasörler aynı ({same} dosya). Eşitlenecek bir şey yok.</p>
  {:else}
    <div class="bar">
      <span class="muted">{diffs.length} fark, {same} aynı dosya{truncated ? " — çok fazla dosya, liste eksik" : ""}</span>
      <span class="spacer"></span>
      <button class="link" onclick={() => setAll((d) => defaultChoice(d.status))}>Öneri</button>
      <button class="link" onclick={() => setAll((d) => (d.status === "onlyRemote" ? "skip" : "upload"))}>Hepsini yükle</button>
      <button class="link" onclick={() => setAll((d) => (d.status === "onlyLocal" ? "skip" : "download"))}>Hepsini indir</button>
      <button class="link" onclick={() => setAll(() => "skip")}>Hiçbiri</button>
    </div>
    <div class="table">
      {#each diffs as d (d.path)}
        <div class="row" class:skip={choices[d.path] === "skip"}>
          <code class="path" title={d.path}>{d.path}</code>
          <span class="st {d.status}">{labels[d.status]}</span>
          <span class="meta">
            {#if d.localSize !== null}{formatSize(d.localSize)} · {formatDate(d.localMtime)}{:else}—{/if}
          </span>
          <span class="meta">
            {#if d.remoteSize !== null}{formatSize(d.remoteSize)} · {formatDate(d.remoteMtime)}{:else}—{/if}
          </span>
          <select bind:value={choices[d.path]} aria-label="Ne yapılsın">
            <option value="upload" disabled={!can(d, "upload")}>→ Yükle</option>
            <option value="download" disabled={!can(d, "download")}>← İndir</option>
            <option value="skip">Atla</option>
          </select>
        </div>
      {/each}
    </div>
    <p class="note">Karşı taraftaki dosyanın üzerine yazılır; hiçbir dosya silinmez.</p>
  {/if}

  {#snippet footer()}
    <button class="btn" onclick={compare} disabled={loading || busy}>Yeniden karşılaştır</button>
    <span class="spacer"></span>
    <button class="btn" onclick={onClose}>Vazgeç</button>
    <button class="btn primary" disabled={!planned.length || busy || loading} onclick={apply}>
      {busy ? "Eşitleniyor…" : `Eşitle (${ups} yükle, ${downs} indir)`}
    </button>
  {/snippet}
</Modal>

<style>
  .roots {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
    color: var(--muted);
  }
  .roots div {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .roots span {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }
  .roots code {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 6px;
    font-size: 12px;
  }
  .spacer {
    flex: 1;
  }
  .link {
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
  }
  .table {
    max-height: 45vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 150px 150px 150px 100px;
    align-items: center;
    gap: 10px;
    padding: 5px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
  }
  .row:last-child {
    border-bottom: 0;
  }
  .row.skip {
    opacity: 0.5;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .st {
    font-size: 11.5px;
  }
  .st.onlyLocal,
  .st.localNewer {
    color: var(--accent);
  }
  .st.onlyRemote,
  .st.remoteNewer {
    color: #61afef;
  }
  .st.different {
    color: var(--warn);
  }
  .meta {
    color: var(--muted);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  select {
    padding: 3px 5px;
    font-size: 12px;
  }
  .muted {
    color: var(--muted);
  }
  .ok {
    color: var(--ok);
  }
  .err {
    color: var(--danger);
  }
  .note {
    margin: 8px 0 0;
    font-size: 11.5px;
    color: var(--muted);
  }
</style>
