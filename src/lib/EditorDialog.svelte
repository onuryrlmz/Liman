<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import Modal from "./Modal.svelte";
  import { api, errText } from "./api";
  import { store } from "./tabs.svelte";

  let { termId, path, onClose }: { termId: string; path: string; onClose: () => void } = $props();

  let text = $state("");
  let original = $state("");
  let loading = $state(true);
  let error = $state("");
  let saving = $state(false);
  const dirty = $derived(text !== original);

  api
    .sftpReadText(termId, path)
    .then((t) => {
      text = original = t;
    })
    .catch((e) => (error = errText(e)))
    .finally(() => (loading = false));

  async function save() {
    saving = true;
    try {
      await api.sftpWriteText(termId, path, text);
      original = text;
      store.notify("Kaydedildi");
    } catch (e) {
      store.notify(errText(e), "error");
    } finally {
      saving = false;
    }
  }

  async function close() {
    if (dirty && !(await ask("Kaydedilmemiş değişiklikler var. Yine de kapatılsın mı?", { kind: "warning" }))) return;
    onClose();
  }

  function onKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "s") {
      e.preventDefault();
      if (dirty) save();
    }
    if (e.key === "Tab") {
      e.preventDefault();
      const ta = e.currentTarget as HTMLTextAreaElement;
      const { selectionStart: s, selectionEnd: en } = ta;
      text = text.slice(0, s) + "\t" + text.slice(en);
      requestAnimationFrame(() => ta.setSelectionRange(s + 1, s + 1));
    }
  }
</script>

<Modal title="{dirty ? '● ' : ''}{path}" width={900} onClose={close}>
  {#if loading}
    <p class="muted">Yükleniyor…</p>
  {:else if error}
    <p class="err">{error}</p>
  {:else}
    <!-- svelte-ignore a11y_autofocus -->
    <textarea bind:value={text} onkeydown={onKey} spellcheck="false" autofocus></textarea>
  {/if}
  {#snippet footer()}
    <span class="muted hint">Cmd/Ctrl+S ile kaydet</span>
    <button class="btn" onclick={close}>Kapat</button>
    <button class="btn primary" disabled={!dirty || saving || !!error} onclick={save}>Kaydet</button>
  {/snippet}
</Modal>

<style>
  textarea {
    width: 100%;
    height: min(62vh, 640px);
    resize: none;
    padding: 10px 12px;
    font-family: "JetBrains Mono", "Cascadia Mono", Menlo, Consolas, monospace;
    font-size: 12.5px;
    line-height: 1.5;
    tab-size: 4;
    white-space: pre;
    background: var(--bg);
  }
  .muted {
    color: var(--muted);
  }
  .hint {
    margin-right: auto;
    align-self: center;
    font-size: 12px;
  }
  .err {
    color: var(--danger);
  }
</style>
