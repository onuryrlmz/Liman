<script lang="ts">
  import { onDestroy } from "svelte";
  import { api, formatSize, type ServerStats } from "./api";
  import { settings } from "./settings.svelte";

  /** Etkin SSH bağlantısı; yoksa çubuk gizlenir. */
  let { termId }: { termId: string | null } = $props();

  let stats = $state<ServerStats | null>(null);
  let failed = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let current: string | null = null;
  let errors = 0;

  async function poll(id: string) {
    if (id !== current) return;
    // Pencere gizliyken sunucuyu yormayalım.
    if (!document.hidden) {
      try {
        const s = await api.sshStats(id);
        if (id !== current) return;
        stats = s;
        errors = 0;
        failed = false;
      } catch {
        if (id !== current) return;
        // Windows sunucular gibi desteklenmeyen sistemlerde sessizce gizlen.
        if (++errors >= 2) failed = true;
      }
    }
    if (id === current) timer = setTimeout(() => poll(id), Math.max(2, settings.value.statsInterval) * 1000);
  }

  $effect(() => {
    const id = settings.value.serverStats ? termId : null;
    if (id === current) return;
    clearTimeout(timer);
    current = id;
    stats = null;
    failed = false;
    errors = 0;
    if (id) poll(id);
  });

  onDestroy(() => {
    current = null;
    clearTimeout(timer);
  });

  const pct = (used: number, total: number) => (total ? (used / total) * 100 : 0);
  const level = (p: number) => (p >= 90 ? "crit" : p >= 70 ? "warn" : "ok");
  const short = (n: number) => formatSize(n).replace(" ", "");
  const rate = (n: number | null) => (n == null ? "–" : `${short(n)}/s`);

  function uptime(s: number) {
    const d = Math.floor(s / 86400);
    const h = Math.floor((s % 86400) / 3600);
    const m = Math.floor((s % 3600) / 60);
    return d ? `${d}g ${h}s` : h ? `${h}s ${m}dk` : `${m}dk`;
  }

  const memP = $derived(stats ? pct(stats.memUsed, stats.memTotal) : 0);
  const diskP = $derived(stats?.disk ? pct(stats.disk.used, stats.disk.total) : 0);
  const loadP = $derived(stats?.load && stats.ncpu ? (stats.load[0] / stats.ncpu) * 100 : 0);
  const diskTitle = $derived(
    stats?.disks.map((d) => `${d.mount}: ${short(d.used)} / ${short(d.total)} (%${Math.round(pct(d.used, d.total))})`).join("\n") ?? "",
  );
</script>

{#if termId && settings.value.serverStats && !failed}
  <span class="stats" class:loading={!stats}>
    {#if !stats}
      <span class="muted">Sunucu ölçülüyor…</span>
    {:else}
      <span class="item" title="İşlemci kullanımı{stats.ncpu ? ` (${stats.ncpu} çekirdek)` : ''}">
        <b>CPU</b>
        {#if stats.cpu != null}
          <span class="bar {level(stats.cpu)}"><span style:width="{stats.cpu}%"></span></span>
          {Math.round(stats.cpu)}%
        {:else}–{/if}
      </span>
      {#if stats.memTotal}
        <span
          class="item"
          title="Bellek: {short(stats.memUsed)} / {short(stats.memTotal)}{stats.swapTotal ? `\nTakas: ${short(stats.swapUsed)} / ${short(stats.swapTotal)}` : ''}"
        >
          <b>RAM</b>
          <span class="bar {level(memP)}"><span style:width="{memP}%"></span></span>
          {short(stats.memUsed)}/{short(stats.memTotal)}
        </span>
      {/if}
      {#if stats.disk}
        <span class="item" title={diskTitle}>
          <b>Disk</b>
          <span class="bar {level(diskP)}"><span style:width="{diskP}%"></span></span>
          {Math.round(diskP)}%
        </span>
      {/if}
      {#if stats.load}
        <span class="item" title="Yük ortalaması (1 / 5 / 15 dk): {stats.load.map((l) => l.toFixed(2)).join(' / ')}">
          <b>Yük</b> <span class={level(loadP)}>{stats.load[0].toFixed(2)}</span>
        </span>
      {/if}
      {#if stats.netRx != null || stats.netTx != null}
        <span class="item" title="Ağ (geri döngü hariç)">↓{rate(stats.netRx)} ↑{rate(stats.netTx)}</span>
      {/if}
      {#if stats.uptime}
        <span class="item" title="Sunucunun açık kalma süresi ({stats.os})"><b>Açık</b> {uptime(stats.uptime)}</span>
      {/if}
    {/if}
  </span>
{/if}

<style>
  .stats {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
    overflow: hidden;
    font-variant-numeric: tabular-nums;
  }
  .item {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    white-space: nowrap;
    cursor: default;
  }
  b {
    font-weight: 600;
    color: var(--text);
    font-size: 10.5px;
    letter-spacing: 0.3px;
  }
  .bar {
    width: 34px;
    height: 5px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    border-radius: 3px;
    transition: width 0.4s;
  }
  .bar.ok span {
    background: var(--ok);
  }
  .bar.warn span {
    background: var(--warn);
  }
  .bar.crit span {
    background: var(--danger);
  }
  span.warn {
    color: var(--warn);
  }
  span.crit {
    color: var(--danger);
  }
  .muted {
    opacity: 0.7;
  }
</style>
