<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import Modal from "./Modal.svelte";
  import Icon from "./Icon.svelte";
  import { api, type SudoQuestion } from "./api";

  // SFTP sudo ile açılırken sudo parola isterse sorulur. Birden fazla bağlantı sırayla.
  let queue = $state<SudoQuestion[]>([]);
  const q = $derived(queue[0]);
  let password = $state("");
  let remember = $state(false);

  onMount(() => {
    const un = listen<SudoQuestion>("sudo-password", (e) => {
      queue.push(e.payload);
    });
    return () => {
      un.then((f) => f());
    };
  });

  function answer(ok: boolean) {
    const cur = queue.shift();
    if (cur) api.sudoAnswer(cur.id, ok ? password : null, ok && remember);
    password = "";
  }
</script>

{#if q}
  <Modal title="sudo parolası" width={420} onClose={() => answer(false)}>
    <form
      class="body"
      onsubmit={(e) => {
        e.preventDefault();
        if (password) answer(true);
      }}
    >
      <div class="icon"><Icon name="key" size={20} /></div>
      <div class="main">
        <p>
          <strong>{q.user}@{q.host}</strong> üzerinde SFTP root yetkisiyle açılıyor. sudo için
          <strong>{q.user}</strong> kullanıcısının parolasını girin.
        </p>
        {#if q.retry}<p class="err">Parola yanlış, tekrar deneyin.</p>{/if}
        <!-- svelte-ignore a11y_autofocus -->
        <input type="password" bind:value={password} autofocus autocomplete="off" />
        <label class="check"><input type="checkbox" bind:checked={remember} /> Hatırla (sistem kasasına kaydet)</label>
      </div>
    </form>
    {#snippet footer()}
      <button class="btn" onclick={() => answer(false)}>Vazgeç</button>
      <button class="btn primary" disabled={!password} onclick={() => answer(true)}>Tamam</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .body {
    display: flex;
    gap: 14px;
  }
  .icon {
    color: var(--accent);
    flex: none;
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 10px;
    flex: 1;
  }
  p {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
  }
  .err {
    color: var(--danger);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
  }
</style>
