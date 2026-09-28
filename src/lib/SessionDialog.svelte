<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import Modal from "./Modal.svelte";
  import { api, errText, type Session } from "./api";
  import { store } from "./tabs.svelte";

  let {
    session,
    onClose,
  }: { session: Session | null; onClose: () => void } = $props();

  const initial: Session = session
    ? { ...session }
    : { id: "", name: "", host: "", port: 22, username: "", auth: "auto", keyPath: "", folder: "" };

  let form = $state(initial);
  let secret = $state("");
  let forget = $state(false);
  let busy = $state(false);
  let error = $state("");

  const folders = $derived(
    [...new Set(store.sessions.map((s) => s.folder).filter((f): f is string => !!f))].sort(),
  );

  async function pickKey() {
    const p = await open({ title: "Özel anahtar seç", multiple: false, directory: false });
    if (typeof p === "string") form.keyPath = p;
  }

  async function save(connect: boolean) {
    error = "";
    if (!form.host.trim()) {
      error = "Sunucu adresi gerekli.";
      return;
    }
    busy = true;
    try {
      const s = await api.sessionSave(
        {
          ...form,
          host: form.host.trim(),
          username: form.username.trim(),
          name: form.name.trim() || `${form.username ? form.username + "@" : ""}${form.host.trim()}`,
          folder: form.folder?.trim() || null,
          keyPath: form.keyPath?.trim() || null,
          port: Number(form.port) || 22,
        },
        forget ? "" : secret ? secret : null,
      );
      await store.loadSessions();
      onClose();
      if (connect) store.openSession(s);
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={session ? "Oturumu düzenle" : "Yeni SSH oturumu"} {onClose}>
  <form
    class="grid"
    onsubmit={(e) => {
      e.preventDefault();
      save(true);
    }}
  >
    <label class="wide">
      <span>Sunucu</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={form.host} placeholder="ornek.com veya 192.168.1.10" autofocus spellcheck="false" />
    </label>
    <label class="narrow">
      <span>Port</span>
      <input type="number" min="1" max="65535" bind:value={form.port} />
    </label>
    <label class="full">
      <span>Kullanıcı adı</span>
      <input bind:value={form.username} placeholder="boş bırakılırsa bağlanırken sorulur" spellcheck="false" />
    </label>
    <label class="full">
      <span>Kimlik doğrulama</span>
      <select bind:value={form.auth}>
        <option value="auto">Otomatik (~/.ssh anahtarları, sonra parola)</option>
        <option value="password">Parola</option>
        <option value="key">Özel anahtar dosyası</option>
      </select>
    </label>
    {#if form.auth === "key"}
      <label class="full">
        <span>Anahtar dosyası</span>
        <div class="row">
          <input bind:value={form.keyPath} placeholder="~/.ssh/id_ed25519" spellcheck="false" />
          <button type="button" class="btn" onclick={pickKey}>Seç…</button>
        </div>
      </label>
    {/if}
    <label class="full">
      <span>{form.auth === "key" ? "Anahtar parolası (varsa)" : "Parola"}</span>
      <input
        type="password"
        bind:value={secret}
        disabled={forget}
        placeholder={session?.hasSecret ? "•••••• (kayıtlı, değiştirmek için yazın)" : "kaydetmek istemiyorsanız boş bırakın"}
      />
      <small>Parolalar işletim sisteminin kasasında (Keychain / Credential Manager / Secret Service) tutulur.</small>
    </label>
    {#if session?.hasSecret}
      <label class="check full">
        <input type="checkbox" bind:checked={forget} /> Kayıtlı parolayı sil
      </label>
    {/if}
    <label>
      <span>Görünen ad</span>
      <input bind:value={form.name} placeholder="isteğe bağlı" />
    </label>
    <label>
      <span>Klasör</span>
      <input bind:value={form.folder} list="folders" placeholder="örn. Üretim" />
      <datalist id="folders">
        {#each folders as f}<option value={f}></option>{/each}
      </datalist>
    </label>
    {#if error}<p class="err full">{error}</p>{/if}
    <button type="submit" hidden aria-hidden="true"></button>
  </form>

  {#snippet footer()}
    <button class="btn" onclick={onClose}>Vazgeç</button>
    <button class="btn" disabled={busy} onclick={() => save(false)}>Kaydet</button>
    <button class="btn primary" disabled={busy} onclick={() => save(true)}>Kaydet ve bağlan</button>
  {/snippet}
</Modal>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 12px;
  }
  .grid > label {
    grid-column: span 2;
  }
  .grid > label.wide {
    grid-column: span 3;
  }
  .grid > label.narrow {
    grid-column: span 1;
  }
  .grid > .full {
    grid-column: 1 / -1;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: var(--muted);
    min-width: 0;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    color: var(--text);
  }
  small {
    color: var(--muted);
    font-size: 11px;
  }
  .row {
    display: flex;
    gap: 6px;
  }
  .row input {
    flex: 1;
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: 13px;
  }
</style>
