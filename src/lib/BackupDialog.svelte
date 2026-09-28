<script lang="ts">
  import { open, save } from "@tauri-apps/plugin-dialog";
  import Modal from "./Modal.svelte";
  import Icon from "./Icon.svelte";
  import ExternalImport from "./ExternalImport.svelte";
  import {
    api,
    errText,
    PASSWORD_REQUIRED,
    WRONG_PASSWORD,
    type BackupFileInfo,
    type ImportResult,
  } from "./api";
  import { store } from "./tabs.svelte";

  let {
    mode: initialMode,
    group: initialGroup = null,
    onClose,
  }: { mode: "export" | "import"; group?: string | null; onClose: () => void } = $props();

  let mode = $state(initialMode);
  let busy = $state(false);
  let error = $state("");

  // --- Dışarı aktarma ---
  let scope = $state<string>(initialGroup ?? "");
  let withSecrets = $state(true);
  let password = $state("");
  let confirm = $state("");
  let showPw = $state(false);

  const scopeCount = $derived(
    scope === "" ? store.sessions.length : store.sessions.filter((s) => s.folder === scope).length,
  );
  const savedSecrets = $derived(
    store.sessions.filter((s) => (scope === "" || s.folder === scope) && s.hasSecret).length,
  );
  const pwError = $derived(
    !withSecrets
      ? ""
      : password.length > 0 && password.length < 8
        ? "En az 8 karakter olmalı."
        : confirm && confirm !== password
          ? "Parolalar eşleşmiyor."
          : "",
  );
  const canExport = $derived(scopeCount > 0 && (!withSecrets || (password.length >= 8 && confirm === password)));

  function today() {
    const d = new Date();
    const p = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
  }

  async function doExport() {
    error = "";
    const slug = scope ? scope.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "-") : "oturumlar";
    const path = await save({
      title: "Oturumları dışarı aktar",
      defaultPath: `liman-${slug}-${today()}.liman`,
      filters: [{ name: "Liman oturumları", extensions: ["liman"] }],
    });
    if (!path) return;
    busy = true;
    try {
      const r = await api.sessionsExport(path, withSecrets ? password : null, scope || null);
      const extra = withSecrets ? `, ${r.secrets} parola, ${r.keys} anahtar` : "";
      store.notify(`${r.sessions} oturum dışarı aktarıldı${extra}.`);
      onClose();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  // --- İçeri aktarma ---
  let importSource = $state<"liman" | "ssh-config" | "mobaxterm">("liman");
  let file = $state<string | null>(null);
  let info = $state<BackupFileInfo | null>(null);
  let importPw = $state("");
  let result = $state<ImportResult | null>(null);

  async function pickFile() {
    error = "";
    result = null;
    const p = await open({
      title: "Liman oturum dosyası seç",
      multiple: false,
      directory: false,
      filters: [{ name: "Liman oturumları", extensions: ["liman", "json"] }],
    });
    if (typeof p !== "string") return;
    try {
      info = await api.sessionsImportInfo(p);
      file = p;
      importPw = "";
    } catch (e) {
      file = null;
      info = null;
      error = errText(e);
    }
  }

  async function doImport() {
    if (!file) return;
    error = "";
    busy = true;
    try {
      result = await api.sessionsImport(file, info?.encrypted ? importPw : null);
      await store.loadSessions();
    } catch (e) {
      const msg = errText(e);
      error = msg.includes(WRONG_PASSWORD)
        ? "Parola yanlış ya da dosya değiştirilmiş."
        : msg.includes(PASSWORD_REQUIRED)
          ? "Bu dosya şifreli, parolasını girin."
          : msg;
    } finally {
      busy = false;
    }
  }

  const fileName = $derived(file?.split(/[\\/]/).pop() ?? "");
</script>

<Modal title="Oturumları aktar" width={480} {onClose}>
  <div class="tabs" role="tablist">
    <button role="tab" aria-selected={mode === "export"} class:on={mode === "export"} onclick={() => ((mode = "export"), (error = ""))}>
      <Icon name="upload" size={14} /> Dışarı aktar
    </button>
    <button role="tab" aria-selected={mode === "import"} class:on={mode === "import"} onclick={() => ((mode = "import"), (error = ""))}>
      <Icon name="download" size={14} /> İçeri aktar
    </button>
  </div>

  {#if mode === "export"}
    <form
      class="form"
      onsubmit={(e) => {
        e.preventDefault();
        if (canExport && !busy) doExport();
      }}
    >
      <label>
        <span>Ne aktarılsın?</span>
        <select bind:value={scope}>
          <option value="">Tüm oturumlar ({store.sessions.length})</option>
          {#each store.groups as g}
            <option value={g}>{g} ({store.sessions.filter((s) => s.folder === g).length})</option>
          {/each}
        </select>
      </label>

      <label class="check">
        <input type="checkbox" bind:checked={withSecrets} />
        <span>
          Parolaları ve özel anahtarları dahil et
          <small>{savedSecrets} kayıtlı parola. Dosyanın tamamı belirlediğiniz parolayla şifrelenir.</small>
        </span>
      </label>

      {#if withSecrets}
        <div class="pw">
          <label>
            <span>Dosya parolası</span>
            <input type={showPw ? "text" : "password"} bind:value={password} autocomplete="new-password" placeholder="en az 8 karakter" />
          </label>
          <label>
            <span>Parola (tekrar)</span>
            <input type={showPw ? "text" : "password"} bind:value={confirm} autocomplete="new-password" />
          </label>
          <label class="check small"><input type="checkbox" bind:checked={showPw} /> Parolayı göster</label>
          {#if pwError}<p class="err">{pwError}</p>{/if}
          <p class="note">
            <Icon name="key" size={13} /> Bu parolayı unutursanız dosya açılamaz; kurtarmanın bir yolu yoktur.
          </p>
        </div>
      {:else}
        <p class="note">Dosya yalnızca sunucu, kullanıcı ve grup bilgilerini içerir; parola ve anahtar içermez.</p>
      {/if}
      {#if error}<p class="err">{error}</p>{/if}
      <button type="submit" hidden aria-hidden="true"></button>
    </form>
  {:else}
    <div class="sources" role="radiogroup" aria-label="Kaynak">
      {#each [["liman", "Liman dosyası"], ["ssh-config", "OpenSSH config"], ["mobaxterm", "MobaXterm"]] as [key, label]}
        <button
          role="radio"
          aria-checked={importSource === key}
          class:on={importSource === key}
          onclick={() => ((importSource = key as typeof importSource), (error = ""))}>{label}</button
        >
      {/each}
    </div>
    {#if importSource !== "liman"}
      {#key importSource}
        <ExternalImport source={importSource} onDone={onClose} />
      {/key}
    {:else}
    <div class="form">
      <button class="file" onclick={pickFile}>
        <Icon name="file" size={18} />
        {#if file}
          <span class="fname">{fileName}</span>
          <span class="meta">
            {info?.encrypted ? "Şifreli · parolalar dahil" : `Şifresiz · ${info?.sessions ?? 0} oturum`}
          </span>
        {:else}
          <span class="fname">Dosya seç…</span>
          <span class="meta">.liman dosyası</span>
        {/if}
      </button>

      {#if file && info?.encrypted && !result}
        <label>
          <span>Dosya parolası</span>
          <!-- svelte-ignore a11y_autofocus -->
          <input
            type="password"
            bind:value={importPw}
            autofocus
            onkeydown={(e) => e.key === "Enter" && importPw && !busy && doImport()}
          />
        </label>
      {/if}

      {#if result}
        <div class="result">
          <strong>İçeri aktarıldı</strong>
          <span>{result.added} oturum eklendi, {result.updated} güncellendi</span>
          {#if result.secrets || result.keys}
            <span>{result.secrets} parola kasaya kaydedildi, {result.keys} anahtar dosyası eklendi</span>
          {/if}
          {#if result.groups}<span>{result.groups} yeni grup</span>{/if}
        </div>
      {:else}
        <p class="note">Aynı oturumlar (daha önce bu dosyadan ya da bu makineden aktarılmış olanlar) güncellenir, diğerleri eklenir. Hiçbir oturum silinmez.</p>
      {/if}
      {#if error}<p class="err">{error}</p>{/if}
    </div>
    {/if}
  {/if}

  {#snippet footer()}
    {#if mode === "export"}
      <button class="btn" onclick={onClose}>Vazgeç</button>
      <button class="btn primary" disabled={!canExport || busy} onclick={doExport}>
        {busy ? "Şifreleniyor…" : `${scopeCount} oturumu dışarı aktar…`}
      </button>
    {:else if importSource !== "liman"}
      <button class="btn" onclick={onClose}>Kapat</button>
    {:else if result}
      <button class="btn primary" onclick={onClose}>Tamam</button>
    {:else}
      <button class="btn" onclick={onClose}>Vazgeç</button>
      <button class="btn primary" disabled={!file || busy || (info?.encrypted && !importPw)} onclick={doImport}>
        {busy ? "Açılıyor…" : "İçeri aktar"}
      </button>
    {/if}
  {/snippet}
</Modal>

<style>
  .tabs {
    display: flex;
    gap: 4px;
    padding: 3px;
    margin-bottom: 16px;
    border-radius: 8px;
    background: var(--bg);
    border: 1px solid var(--border);
  }
  .tabs button {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    padding: 7px;
    border: 0;
    border-radius: 6px;
    background: none;
    color: var(--muted);
    font: inherit;
    cursor: pointer;
  }
  .tabs button.on {
    background: var(--panel-2);
    color: var(--text);
  }
  .sources {
    display: flex;
    gap: 6px;
    margin-bottom: 14px;
  }
  .sources button {
    padding: 5px 11px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .sources button.on {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: var(--muted);
  }
  label.check {
    flex-direction: row;
    align-items: flex-start;
    gap: 9px;
    color: var(--text);
    font-size: 13px;
  }
  label.check input {
    margin-top: 2px;
  }
  label.check small {
    display: block;
    margin-top: 2px;
    color: var(--muted);
    font-size: 11.5px;
  }
  label.small {
    font-size: 12px;
    align-items: center;
  }
  .pw {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px 12px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
  }
  .pw input {
    background: var(--panel);
  }
  .pw > :global(.check),
  .pw > .err,
  .pw > .note {
    grid-column: 1 / -1;
  }
  .note {
    display: flex;
    gap: 7px;
    align-items: flex-start;
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--muted);
  }
  .note :global(svg) {
    flex: none;
    margin-top: 2px;
    color: var(--warn);
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: 12.5px;
  }
  .file {
    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-rows: auto auto;
    column-gap: 12px;
    align-items: center;
    padding: 14px 16px;
    border: 1px dashed var(--border);
    border-radius: 8px;
    background: var(--bg);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .file:hover {
    border-color: var(--accent);
  }
  .file :global(svg) {
    grid-row: 1 / 3;
    color: var(--accent);
  }
  .fname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    font-size: 11.5px;
    color: var(--muted);
  }
  .result {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 12px 14px;
    border-radius: 8px;
    border: 1px solid var(--ok);
    background: rgb(152 195 121 / 0.08);
    font-size: 12.5px;
  }
  .result strong {
    color: var(--ok);
  }
</style>
