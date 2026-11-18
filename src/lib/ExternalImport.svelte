<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import { api, errText, MREMOTENG_PASSWORD_REQUIRED, type ImportSource, type Session } from "./api";
  import { store } from "./tabs.svelte";

  let { source, onDone }: { source: ImportSource; onDone: () => void } = $props();

  const fileSources: Record<"mobaxterm" | "mremoteng", { title: string; filters: { name: string; extensions: string[] }[] }> = {
    mobaxterm: { title: "MobaXterm oturum dosyası", filters: [{ name: "MobaXterm", extensions: ["mxtsessions", "ini"] }] },
    mremoteng: { title: "mRemoteNG bağlantı dosyası", filters: [{ name: "mRemoteNG", extensions: ["xml"] }] },
  };

  let sessions = $state<Session[]>([]);
  let unsupported = $state(0);
  let lockedPasswords = $state(0);
  let needsPassword = $state(false);
  let path = $state<string | null>(null);
  let filePw = $state("");
  let selected = $state<Set<string>>(new Set());
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state("");
  let result = $state<{ added: number; skipped: number } | null>(null);

  const existingKeys = $derived(new Set(store.sessions.map((s) => `${s.host.toLowerCase()}|${s.port}|${s.username}|${s.kind ?? "ssh"}`)));
  const isDup = (s: Session) => existingKeys.has(`${s.host.toLowerCase()}|${s.port}|${s.username}|${s.kind ?? "ssh"}`);

  async function load(file: string | null, password: string | null = null) {
    error = "";
    result = null;
    busy = true;
    path = file;
    try {
      const p = await api.importPreview(source, file, password);
      sessions = p.sessions;
      unsupported = p.unsupported;
      lockedPasswords = p.lockedPasswords;
      needsPassword = p.needsPassword;
      selected = new Set(p.sessions.filter((s) => !isDup(s)).map((s) => s.id));
      loaded = true;
    } catch (e) {
      const msg = errText(e);
      if (msg.includes(MREMOTENG_PASSWORD_REQUIRED)) {
        // Tüm dosya şifreli: oturumları görmek için bile parola gerekir.
        sessions = [];
        needsPassword = true;
        loaded = false;
      } else {
        error = msg;
      }
    } finally {
      busy = false;
    }
  }

  async function pick() {
    const fs = source === "ssh-config" ? null : fileSources[source];
    const p = await open({
      title: fs?.title ?? "SSH config dosyası",
      multiple: false,
      directory: false,
      filters: fs?.filters,
    });
    if (typeof p === "string") {
      filePw = "";
      load(p);
    }
  }

  function unlock(e: SubmitEvent) {
    e.preventDefault();
    if (filePw) load(path, filePw);
  }

  const kindTag: Record<string, string> = { sftp: "SFTP", rdp: "RDP", vnc: "VNC", telnet: "Telnet" };

  function toggle(id: string) {
    const s = new Set(selected);
    s.has(id) ? s.delete(id) : s.add(id);
    selected = s;
  }

  async function run() {
    busy = true;
    error = "";
    try {
      result = await api.importSessions(sessions.filter((s) => selected.has(s.id)));
      await store.loadSessions();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  // ~/.ssh/config varsayılan yerindeyse hemen oku.
  if (source === "ssh-config") load(null);
</script>

<div class="ext">
  {#if source === "mobaxterm"}
    <p class="note">
      MobaXterm'in oturum listesinden dışa aktardığınız <code>.mxtsessions</code> dosyasını ya da
      <code>MobaXterm.ini</code>'yi seçin. SSH ve SFTP oturumları aktarılır;
      parolalar aktarılmaz.
    </p>
  {:else if source === "mremoteng"}
    <p class="note">
      mRemoteNG'nin bağlantı dosyasını (<code>confCons.xml</code> ya da dışa aktarılmış <code>.xml</code>) seçin.
      SSH, RDP, VNC ve Telnet bağlantıları klasörleriyle aktarılır. Parolalar Liman'ın parola kasasına kaydedilir;
      dosya mRemoteNG'de bir parolayla korunuyorsa o parola sorulur.
    </p>
  {:else}
    <p class="note">
      <code>~/.ssh/config</code> içindeki <code>Host</code> kayıtları; HostName, User, Port, IdentityFile ve ProxyJump
      dahil. Joker (<code>*</code>) kayıtlar varsayılan olarak uygulanır.
    </p>
  {/if}

  <button class="btn" onclick={pick} disabled={busy}>
    <Icon name="file" size={14} />
    {source === "ssh-config" ? "Başka bir config dosyası seç…" : "Dosya seç…"}
  </button>

  {#if needsPassword && !result}
    <form class="pw" onsubmit={unlock}>
      <span>
        {loaded
          ? `Dosya parolayla korunuyor; ${lockedPasswords} oturumun parolası için mRemoteNG dosya parolasını girin. Girmezseniz oturumlar parolasız aktarılır.`
          : "Dosyanın tamamı şifreli; oturumları görmek için mRemoteNG dosya parolasını girin."}
      </span>
      <div class="pwrow">
        <input type="password" placeholder="mRemoteNG dosya parolası" bind:value={filePw} disabled={busy} />
        <button class="btn" type="submit" disabled={busy || !filePw}>Çöz</button>
      </div>
    </form>
  {/if}

  {#if result}
    <div class="result">
      <strong>{result.added} oturum eklendi</strong>
      {#if result.skipped}<span>{result.skipped} oturum zaten kayıtlı olduğu için atlandı</span>{/if}
    </div>
  {:else if loaded}
    {#if sessions.length}
      <div class="head">
        <span
          >{sessions.length} oturum bulundu{unsupported
            ? `, ${unsupported} desteklenmeyen (${source === "mremoteng" ? "Rlogin, HTTP…" : "RDP, VNC…"}) atlandı`
            : ""}{sessions.some((s) => s.hasSecret) ? `, ${sessions.filter((s) => s.hasSecret).length} parolalı` : ""}</span
        >
        <button class="link" onclick={() => (selected = selected.size === sessions.length ? new Set() : new Set(sessions.map((s) => s.id)))}>
          {selected.size === sessions.length ? "Hiçbirini seçme" : "Tümünü seç"}
        </button>
      </div>
      <div class="list">
        {#each sessions as s (s.id)}
          <label class="row" class:dup={isDup(s)}>
            <input type="checkbox" checked={selected.has(s.id)} onchange={() => toggle(s.id)} />
            <span class="name">{s.name}</span>
            <span class="host"
              >{s.folder && source === "mremoteng" ? s.folder + " · " : ""}{s.username ? s.username + "@" : ""}{s.host}{s.port !== 22 ? ":" + s.port : ""}</span
            >
            {#if s.kind && kindTag[s.kind]}<span class="tag">{kindTag[s.kind]}</span>{/if}
            {#if s.hasSecret}<span class="tag" title="Parola da aktarılacak"><Icon name="key" size={10} /></span>{/if}
            {#if s.jump}<span class="tag" title="Atlama sunucusu var">↪</span>{/if}
            {#if isDup(s)}<span class="tag">kayıtlı</span>{/if}
          </label>
        {/each}
      </div>
    {:else}
      <p class="note">Aktarılabilecek oturum bulunamadı{unsupported ? ` (${unsupported} desteklenmeyen tür atlandı)` : ""}.</p>
    {/if}
  {/if}
  {#if error}<p class="err">{error}</p>{/if}

  <div class="actions">
    {#if result}
      <button class="btn primary" onclick={onDone}>Tamam</button>
    {:else}
      <button class="btn primary" disabled={busy || !selected.size} onclick={run}>{selected.size} oturumu içe aktar</button>
    {/if}
  </div>
</div>

<style>
  .ext {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .ext > .btn {
    align-self: flex-start;
  }
  .note {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--muted);
  }
  .head {
    display: flex;
    justify-content: space-between;
    font-size: 12px;
    color: var(--muted);
  }
  .link {
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
  }
  .list {
    max-height: 260px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    padding: 4px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-radius: 5px;
    font-size: 12.5px;
    cursor: pointer;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.dup {
    opacity: 0.6;
  }
  .name {
    flex: none;
    max-width: 40%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .host {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
    font-size: 11.5px;
  }
  .tag {
    flex: none;
    padding: 0 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 10px;
    color: var(--muted);
  }
  .pw {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border: 1px solid var(--warn);
    border-radius: 8px;
    font-size: 12.5px;
    line-height: 1.5;
  }
  .pwrow {
    display: flex;
    gap: 8px;
  }
  .pwrow input {
    flex: 1;
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
  .err {
    margin: 0;
    color: var(--danger);
    font-size: 12.5px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
  }
</style>
