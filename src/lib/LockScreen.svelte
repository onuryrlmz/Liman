<script lang="ts">
  import Icon from "./Icon.svelte";
  import { api, errText } from "./api";
  import { store } from "./tabs.svelte";

  // Ana parola tanımlıyken açılışta gösterilir; oturumlar kilit açılınca yüklenir.
  let password = $state("");
  let error = $state("");
  let busy = $state(false);

  async function unlock(e: SubmitEvent) {
    e.preventDefault();
    if (!password || busy) return;
    busy = true;
    error = "";
    try {
      await api.vaultUnlock(password);
      await store.loadVault();
      await store.loadSessions();
    } catch (err) {
      error = errText(err);
      password = "";
    } finally {
      busy = false;
    }
  }
</script>

<div class="lock">
  <form class="card" onsubmit={unlock}>
    <div class="logo"><Icon name="anchor" size={30} /></div>
    <h1>Liman kilitli</h1>
    <p>Oturumlarınızı ve kayıtlı parolalarınızı açmak için ana parolayı girin.</p>
    <!-- svelte-ignore a11y_autofocus -->
    <input type="password" bind:value={password} placeholder="Ana parola" autocomplete="current-password" autofocus />
    {#if error}<p class="err">{error}</p>{/if}
    <button class="btn primary" type="submit" disabled={!password || busy}>{busy ? "Açılıyor…" : "Kilidi aç"}</button>
    <p class="hint">Ana parolayı unuttuysanız kayıtlı oturumlar ve parolalar açılamaz.</p>
  </form>
</div>

<style>
  .lock {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: grid;
    place-items: center;
    padding: 16px;
    background: var(--bg);
  }
  .card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    width: min(360px, 100%);
    text-align: center;
  }
  .logo {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    border-radius: 14px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  h1 {
    margin: 4px 0 0;
    font-size: 18px;
  }
  p {
    margin: 0;
    font-size: 13px;
    color: var(--muted);
    line-height: 1.5;
  }
  input,
  .btn {
    width: 100%;
  }
  .btn {
    justify-content: center;
  }
  input {
    padding: 8px 10px;
    font-size: 14px;
  }
  .err {
    color: var(--danger);
  }
  .hint {
    font-size: 11.5px;
  }
</style>
