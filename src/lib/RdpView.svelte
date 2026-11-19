<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { openPath } from "@tauri-apps/plugin-opener";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import Icon from "./Icon.svelte";
  import { api, errText } from "./api";
  import { openVia } from "./via";
  import type { Pane, TabPatch } from "./tabs.svelte";

  let {
    tab,
    onUpdate,
    onFocus,
  }: { tab: Pane; visible: boolean; focused: boolean; onUpdate: (p: TabPatch) => void; onFocus: () => void } = $props();

  let phase = $state<"connecting" | "open" | "error">("connecting");
  let message = $state("");
  let target = $state("");
  let file = $state("");
  let viaId: string | null = null;
  // Parola durumu: istemciye verildi (Windows) ya da panoya kopyalandı (macOS/Linux).
  let pw = $state<"none" | "given" | "copied">("none");
  let hasPassword = $state(false);
  let clearTimer: ReturnType<typeof setTimeout> | undefined;
  const pasteKey = navigator.platform.toLowerCase().includes("mac") ? "⌘V" : "Ctrl+V";

  /** Kayıtlı parolayı panoya kopyalar; 60 sn sonra pano hâlâ onu içeriyorsa temizler. */
  async function copyPassword() {
    if (!tab.sessionId) return;
    const secret = await api.sessionPassword(tab.sessionId).catch(() => null);
    if (!secret) return;
    await writeText(secret);
    pw = "copied";
    clearTimeout(clearTimer);
    clearTimer = setTimeout(async () => {
      if ((await readText().catch(() => null)) === secret) await writeText("");
    }, 60_000);
  }
  const isMac = navigator.platform.toLowerCase().includes("mac");
  const clientHint = navigator.platform.startsWith("Win")
    ? "Windows'un Uzak Masaüstü Bağlantısı (mstsc) açılır."
    : isMac
      ? "Mac'te Microsoft'un ücretsiz \"Windows App\" uygulaması (App Store) gerekir."
      : "Linux'ta Remmina ya da xfreerdp gibi bir RDP istemcisi gerekir.";

  async function launch() {
    phase = "connecting";
    message = "";
    onUpdate({ status: "connecting" });
    const { host, port, username, jump } = tab.connect!;
    try {
      // Her açılışta taze tünel: önceki SSH bağlantısı kopmuş olabilir, açık kalanı da sızmasın.
      if (viaId) api.termClose(viaId);
      viaId = null;
      if (jump) viaId = await openVia(jump, () => {});
      if (destroyed) {
        if (viaId) api.termClose(viaId);
        return;
      }
      const r = await api.rdpPrepare(host, port, username, viaId, tab.sessionId ?? null);
      file = r.file;
      target = r.target;
      hasPassword = r.hasPassword;
      pw = r.passwordGiven ? "given" : "none";
      if (r.hasPassword && !r.passwordGiven) await copyPassword();
      await openPath(file);
      phase = "open";
      onUpdate({ status: "open" });
    } catch (e) {
      phase = "error";
      message = errText(e);
      onUpdate({ status: "error" });
    }
  }

  export function restart() {
    launch();
  }

  onMount(launch);
  let destroyed = false;
  onDestroy(() => {
    destroyed = true;
    clearTimeout(clearTimer);
    if (viaId) api.termClose(viaId);
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="rdp" onmousedown={onFocus}>
  <div class="card">
    <Icon name="monitor" size={30} />
    <h2>Uzak Masaüstü (RDP)</h2>
    <code>{tab.connect?.username ? tab.connect.username + "@" : ""}{tab.connect?.host}:{tab.connect?.port}</code>
    {#if phase === "connecting"}
      <p>Hazırlanıyor…</p>
    {:else if phase === "error"}
      <p class="err">{message}</p>
      <p class="muted">{clientHint}</p>
    {:else}
      <p>Bağlantı bilgisayarınızdaki RDP istemcisinde açıldı.</p>
      {#if pw === "given"}
        <p class="muted">Kayıtlı parola istemciye verildi.</p>
      {:else if pw === "copied"}
        <p class="muted">Kayıtlı parola panoya kopyalandı; istemci parola sorunca {pasteKey} ile yapıştırın. 60 saniye sonra panodan silinir.</p>
      {/if}
      {#if viaId}
        <p class="muted">
          SSH tüneli açık: istemci <code>{target}</code> adresine bağlanıyor. Tünel bu sekme açık kaldıkça çalışır.
        </p>
      {/if}
      <p class="muted">{clientHint}</p>
    {/if}
    <div class="actions">
      <button class="btn primary" onclick={launch} disabled={phase === "connecting"}>Yeniden aç</button>
      {#if target}
        <button class="btn" onclick={() => writeText(target)}>Adresi kopyala</button>
      {/if}
      {#if hasPassword}
        <button class="btn" onclick={copyPassword}>Parolayı kopyala</button>
      {/if}
    </div>
  </div>
</div>

<style>
  .rdp {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: var(--panel);
    padding: 24px;
  }
  .card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    max-width: 480px;
    text-align: center;
  }
  .card :global(svg) {
    color: var(--accent);
  }
  h2 {
    margin: 0;
    font-size: 17px;
  }
  p {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
  }
  .muted {
    color: var(--muted);
    font-size: 12.5px;
  }
  .err {
    color: var(--danger);
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }
</style>
