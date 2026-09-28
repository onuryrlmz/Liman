<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import { api, errText, type Session } from "./api";
  import { store } from "./tabs.svelte";

  let { source, onDone }: { source: "ssh-config" | "mobaxterm"; onDone: () => void } = $props();

  let sessions = $state<Session[]>([]);
  let unsupported = $state(0);
  let selected = $state<Set<string>>(new Set());
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state("");
  let result = $state<{ added: number; skipped: number } | null>(null);

  const existingKeys = $derived(new Set(store.sessions.map((s) => `${s.host.toLowerCase()}|${s.port}|${s.username}|${s.kind ?? "ssh"}`)));
  const isDup = (s: Session) => existingKeys.has(`${s.host.toLowerCase()}|${s.port}|${s.username}|${s.kind ?? "ssh"}`);

  async function load(path: string | null) {
    error = "";
    result = null;
    busy = true;
    try {
      const p = await api.importPreview(source, path);
      sessions = p.sessions;
      unsupported = p.unsupported;
      selected = new Set(p.sessions.filter((s) => !isDup(s)).map((s) => s.id));
      loaded = true;
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function pick() {
    const p = await open({
      title: source === "mobaxterm" ? "MobaXterm oturum dosyası" : "SSH config dosyası",
      multiple: false,
      directory: false,
      filters: source === "mobaxterm" ? [{ name: "MobaXterm", extensions: ["mxtsessions", "ini"] }] : undefined,
    });
    if (typeof p === "string") load(p);
  }

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
  {:else}
    <p class="note">
      <code>~/.ssh/config</code> içindeki <code>Host</code> kayıtları; HostName, User, Port, IdentityFile ve ProxyJump
      dahil. Joker (<code>*</code>) kayıtlar varsayılan olarak uygulanır.
    </p>
  {/if}

  <button class="btn" onclick={pick} disabled={busy}>
    <Icon name="file" size={14} />
    {source === "mobaxterm" ? "Dosya seç…" : "Başka bir config dosyası seç…"}
  </button>

  {#if result}
    <div class="result">
      <strong>{result.added} oturum eklendi</strong>
      {#if result.skipped}<span>{result.skipped} oturum zaten kayıtlı olduğu için atlandı</span>{/if}
    </div>
  {:else if loaded}
    {#if sessions.length}
      <div class="head">
        <span>{sessions.length} oturum bulundu{unsupported ? `, ${unsupported} desteklenmeyen (RDP, VNC…) atlandı` : ""}</span>
        <button class="link" onclick={() => (selected = selected.size === sessions.length ? new Set() : new Set(sessions.map((s) => s.id)))}>
          {selected.size === sessions.length ? "Hiçbirini seçme" : "Tümünü seç"}
        </button>
      </div>
      <div class="list">
        {#each sessions as s (s.id)}
          <label class="row" class:dup={isDup(s)}>
            <input type="checkbox" checked={selected.has(s.id)} onchange={() => toggle(s.id)} />
            <span class="name">{s.name}</span>
            <span class="host">{s.username ? s.username + "@" : ""}{s.host}{s.port !== 22 ? ":" + s.port : ""}</span>
            {#if s.kind === "sftp"}<span class="tag">SFTP</span>{/if}
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
