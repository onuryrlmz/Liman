<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import Modal from "./Modal.svelte";
  import Icon from "./Icon.svelte";
  import { api, type HostKeyQuestion } from "./api";

  // Aynı anda birden fazla bağlantı soru sorabilir; sırayla gösterilir.
  let queue = $state<HostKeyQuestion[]>([]);
  const q = $derived(queue[0]);

  onMount(() => {
    const un = listen<HostKeyQuestion>("host-key", (e) => {
      queue.push(e.payload);
    });
    return () => {
      un.then((f) => f());
    };
  });

  function answer(accept: boolean) {
    const cur = queue.shift();
    if (cur) api.hostKeyAnswer(cur.id, accept);
  }

  const target = $derived(q ? (q.port === 22 ? q.host : `${q.host}:${q.port}`) : "");
</script>

{#if q}
  <Modal title={q.changed ? "Sunucu anahtarı DEĞİŞMİŞ" : "Yeni sunucu"} width={500} onClose={() => answer(false)}>
    <div class="body" class:danger={q.changed}>
      <div class="icon"><Icon name="key" size={22} /></div>
      {#if q.changed}
        <p>
          <strong>{target}</strong> sunucusunun anahtarı daha önce kaydedilenden <strong>farklı</strong>.
        </p>
        <p class="warn">
          Bu, sunucu yeniden kurulduysa normaldir. Ancak biri bağlantınızı dinliyor da olabilir
          (ortadaki adam saldırısı). Sunucu yöneticisiyle doğrulamadan devam etmeyin.
        </p>
      {:else}
        <p>
          <strong>{target}</strong> sunucusuna ilk kez bağlanıyorsunuz. Anahtarının parmak izi aşağıda; sunucu
          yöneticisinin verdiğiyle aynıysa güvenebilirsiniz.
        </p>
      {/if}
      <div class="fp">
        <span class="alg">{q.algorithm}</span>
        <code>{q.fingerprint}</code>
        <button class="icon-btn" title="Kopyala" onclick={() => writeText(q.fingerprint)}><Icon name="copy" size={13} /></button>
      </div>
      {#if queue.length > 1}<p class="more">{queue.length - 1} soru daha bekliyor.</p>{/if}
    </div>
    {#snippet footer()}
      <button class="btn" onclick={() => answer(false)}>Bağlanma</button>
      {#if q.changed}
        <button class="btn danger" onclick={() => answer(true)}>Yeni anahtarı kabul et ve bağlan</button>
      {:else}
        <!-- svelte-ignore a11y_autofocus -->
        <button class="btn primary" autofocus onclick={() => answer(true)}>Güven ve bağlan</button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: 13px;
    line-height: 1.55;
    user-select: text;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 42px;
    height: 42px;
    border-radius: 10px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .danger .icon {
    background: rgb(255 122 133 / 0.15);
    color: var(--danger);
  }
  p {
    margin: 0;
  }
  .warn {
    padding: 10px 12px;
    border-radius: 8px;
    border: 1px solid var(--danger);
    background: rgb(255 122 133 / 0.08);
    color: var(--danger);
  }
  .fp {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
  }
  .fp code {
    flex: 1;
    word-break: break-all;
  }
  .alg {
    flex: none;
    font-size: 11px;
    color: var(--muted);
  }
  .more {
    font-size: 12px;
    color: var(--muted);
  }
  :global(.btn.danger) {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
