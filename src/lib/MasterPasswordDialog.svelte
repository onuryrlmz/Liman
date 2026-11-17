<script lang="ts">
  import Modal from "./Modal.svelte";
  import { api, errText } from "./api";
  import { store } from "./tabs.svelte";

  let { mode, onClose }: { mode: "enable" | "change" | "disable"; onClose: () => void } = $props();

  let current = $state("");
  let next = $state("");
  let confirm = $state("");
  let error = $state("");
  let busy = $state(false);

  const title = $derived({ enable: "Ana parola belirle", change: "Ana parolayı değiştir", disable: "Ana parolayı kaldır" }[mode]);
  const needsNew = $derived(mode !== "disable");
  const needsCurrent = $derived(mode !== "enable");
  const problem = $derived(
    needsNew && next.length > 0 && next.length < 8
      ? "En az 8 karakter olmalı"
      : needsNew && confirm && confirm !== next
        ? "Parolalar aynı değil"
        : "",
  );
  const ready = $derived(
    !busy && !problem && (!needsCurrent || current.length > 0) && (!needsNew || (next.length >= 8 && confirm === next)),
  );

  async function submit(e?: SubmitEvent) {
    e?.preventDefault();
    if (!ready) return;
    busy = true;
    error = "";
    try {
      if (mode === "enable") await api.vaultEnable(next);
      else if (mode === "change") await api.vaultChange(current, next);
      else await api.vaultDisable(current);
      await store.loadVault();
      await store.loadSessions();
      store.notify(
        mode === "enable" ? "Ana parola belirlendi" : mode === "change" ? "Ana parola değiştirildi" : "Ana parola kaldırıldı",
      );
      onClose();
    } catch (err) {
      error = errText(err);
    } finally {
      busy = false;
    }
  }
</script>

<Modal {title} width={420} {onClose}>
  <form class="form" onsubmit={submit}>
    {#if mode === "enable"}
      <p class="note">
        Oturumlarınız, grupları ve kayıtlı parolalarınız bu parolayla şifrelenmiş tek bir dosyada tutulur. Liman her
        açılışta ve dışarı aktarırken ana parolayı sorar.
      </p>
    {:else if mode === "disable"}
      <p class="note">Oturumlar ve parolalar yeniden sistem kasasına ve düz dosyalara taşınır.</p>
    {/if}
    {#if needsCurrent}
      <label>
        <span>{mode === "change" ? "Şu anki ana parola" : "Ana parola"}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="password" bind:value={current} autocomplete="current-password" autofocus />
      </label>
    {/if}
    {#if needsNew}
      <label>
        <span>{mode === "change" ? "Yeni ana parola" : "Ana parola"}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="password" bind:value={next} autocomplete="new-password" placeholder="en az 8 karakter" autofocus={mode === "enable"} />
      </label>
      <label>
        <span>Parola (tekrar)</span>
        <input type="password" bind:value={confirm} autocomplete="new-password" />
      </label>
      <p class="warn">Ana parolayı unutursanız kayıtlı oturumlar ve parolalar kurtarılamaz.</p>
    {/if}
    {#if problem || error}<p class="err">{problem || error}</p>{/if}
    <button type="submit" hidden aria-hidden="true"></button>
  </form>
  {#snippet footer()}
    <button class="btn" onclick={onClose}>Vazgeç</button>
    <button class="btn {mode === 'disable' ? 'danger' : 'primary'}" disabled={!ready} onclick={() => submit()}>
      {busy ? "Bekleyin…" : mode === "disable" ? "Kaldır" : "Kaydet"}
    </button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12.5px;
    color: var(--muted);
  }
  p {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.5;
  }
  .note {
    color: var(--muted);
  }
  .warn {
    color: var(--warn);
  }
  .err {
    color: var(--danger);
  }
</style>
