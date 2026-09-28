<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import { updater } from "./update-store.svelte";
  import { formatSize } from "./api";

  let version = $state("");
  getVersion().then((v) => (version = v)).catch(() => {});
  const st = $derived(updater.state);
</script>

<div class="updater">
  <div class="line">
    <span>Kurulu sürüm: <strong>{version}</strong></span>
    {#if st.kind === "available"}
      <button class="btn primary" onclick={() => updater.install()}>{st.version} sürümünü kur</button>
    {:else if st.kind === "ready"}
      <button class="btn primary" onclick={() => updater.restart()}>Yeniden başlat</button>
    {:else}
      <button class="btn" disabled={st.kind === "checking" || st.kind === "downloading"} onclick={() => updater.check()}>
        {st.kind === "checking" ? "Denetleniyor…" : "Güncellemeleri denetle"}
      </button>
    {/if}
  </div>
  {#if st.kind === "latest"}
    <p class="ok">Liman güncel.</p>
  {:else if st.kind === "available"}
    <p>Yeni sürüm hazır: <strong>{st.version}</strong></p>
    {#if st.notes}<pre class="notes">{st.notes}</pre>{/if}
  {:else if st.kind === "downloading"}
    <div class="bar"><span style:width="{st.total ? (st.done / st.total) * 100 : 5}%"></span></div>
    <p class="muted">İndiriliyor… {formatSize(st.done)}{st.total ? ` / ${formatSize(st.total)}` : ""}</p>
  {:else if st.kind === "ready"}
    <p class="ok">Güncelleme kuruldu. Açık bağlantılar yeniden başlatınca kapanır.</p>
  {:else if st.kind === "error"}
    <p class="err">Güncelleme denetlenemedi: {st.message}</p>
  {/if}
</div>

<style>
  .updater {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    font-size: 13px;
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  p {
    margin: 0;
    font-size: 12.5px;
  }
  .ok {
    color: var(--ok);
  }
  .err {
    color: var(--danger);
    word-break: break-word;
  }
  .muted {
    color: var(--muted);
  }
  .notes {
    margin: 0;
    max-height: 140px;
    overflow: auto;
    padding: 8px;
    border-radius: 6px;
    background: var(--panel);
    font: 12px/1.5 inherit;
    white-space: pre-wrap;
  }
  .bar {
    height: 5px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.2s;
  }
</style>
