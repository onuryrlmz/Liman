<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import Modal from "./Modal.svelte";
  import { api, errText, type Session } from "./api";
  import { store } from "./tabs.svelte";

  let {
    session,
    folder = null,
    onClose,
  }: { session: Session | null; folder?: string | null; onClose: () => void } = $props();

  const initial: Session = session
    ? { ...session }
    : { id: "", name: "", host: "", port: 22, username: "", auth: "auto", keyPath: "", folder: folder ?? "", kind: "ssh" };

  let form = $state(initial);
  let secret = $state("");
  let forget = $state(false);
  let busy = $state(false);
  let error = $state("");

  const folders = $derived(store.groups);

  // Atlama sunucusu: kayıtlı bir oturum ya da elle yazılan "kullanıcı@sunucu:port".
  const jumpCandidates = $derived(store.sessions.filter((s) => s.id !== form.id && s.kind !== "sftp"));
  let jumpMode = $state<string>(
    !initial.jump ? "" : store.sessions.some((s) => s.id === initial.jump) ? initial.jump : "custom",
  );
  let jumpCustom = $state(jumpMode === "custom" ? (initial.jump ?? "") : "");

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
          jump: jumpMode === "custom" ? jumpCustom.trim() || null : jumpMode || null,
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

<Modal title={session ? "Oturumu düzenle" : "Yeni oturum"} {onClose}>
  <form
    class="grid"
    onsubmit={(e) => {
      e.preventDefault();
      save(true);
    }}
  >
    <div class="kind full" role="radiogroup" aria-label="Bağlantı türü">
      <button type="button" role="radio" aria-checked={form.kind !== "sftp"} class:on={form.kind !== "sftp"} onclick={() => (form.kind = "ssh")}>
        <strong>SSH</strong>
        <span>Terminal + yanda SFTP paneli</span>
      </button>
      <button type="button" role="radio" aria-checked={form.kind === "sftp"} class:on={form.kind === "sftp"} onclick={() => (form.kind = "sftp")}>
        <strong>Yalnızca SFTP</strong>
        <span>Terminal açmadan dosya tarayıcısı</span>
      </button>
    </div>
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
      <span>Grup</span>
      <input bind:value={form.folder} list="folders" placeholder="örn. Üretim" />
      <datalist id="folders">
        {#each folders as f}<option value={f}></option>{/each}
      </datalist>
    </label>
    <label class="full">
      <span>Atlama sunucusu (ProxyJump)</span>
      <select bind:value={jumpMode}>
        <option value="">Yok, doğrudan bağlan</option>
        {#each jumpCandidates as j (j.id)}
          <option value={j.id}>{j.name} — {j.username ? j.username + "@" : ""}{j.host}</option>
        {/each}
        <option value="custom">Diğer (kullanıcı@sunucu:port)…</option>
      </select>
    </label>
    {#if jumpMode === "custom"}
      <label class="full">
        <span>Atlama sunucusu adresi</span>
        <input bind:value={jumpCustom} placeholder="ops@bastion.example.com:22  (zincir için virgülle ayırın)" spellcheck="false" />
        <small>Bu sunucuya ssh-agent ya da ~/.ssh anahtarlarıyla bağlanılır. Parola gerekiyorsa onu ayrı bir oturum olarak kaydedip listeden seçin.</small>
      </label>
    {/if}
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
  .kind {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .kind button {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 9px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .kind button span {
    font-size: 11.5px;
    color: var(--muted);
  }
  .kind button.on {
    border-color: var(--accent);
    background: var(--accent-soft);
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
