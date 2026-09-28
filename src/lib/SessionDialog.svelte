<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import Modal from "./Modal.svelte";
  import { api, errText, type Session, type SessionKind } from "./api";
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

  const isSsh = $derived(form.kind === "ssh" || form.kind === "sftp" || !form.kind);
  const kinds: { key: SessionKind; title: string; desc: string }[] = [
    { key: "ssh", title: "SSH", desc: "Terminal + yanda SFTP" },
    { key: "sftp", title: "Yalnızca SFTP", desc: "Dosya tarayıcısı" },
    { key: "telnet", title: "Telnet", desc: "Ağ cihazları, eski sistemler" },
    { key: "serial", title: "Seri port", desc: "USB-seri, konsol kablosu" },
    { key: "vnc", title: "VNC", desc: "Uzak masaüstü, uygulama içinde" },
    { key: "rdp", title: "RDP", desc: "Windows uzak masaüstü" },
  ];
  const defaultPorts: Partial<Record<SessionKind, number>> = { ssh: 22, sftp: 22, telnet: 23, vnc: 5900, rdp: 3389 };
  const isDesktop = $derived(form.kind === "vnc" || form.kind === "rdp");
  const bauds = [300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

  function setKind(k: SessionKind) {
    const prevDefault = defaultPorts[form.kind ?? "ssh"];
    form.kind = k;
    // Varsayılan portu türe göre ayarla (kullanıcı değiştirmediyse).
    if (defaultPorts[k] && (!form.port || form.port === prevDefault)) form.port = defaultPorts[k]!;
    // VNC/RDP'de "atlama sunucusu" SSH tüneli anlamına gelir; SSH'den gelen değeri taşıma.
    if (k === "vnc" || k === "rdp") jumpMode = jumpMode === "custom" ? "custom" : jumpMode;
    if (k === "serial") {
      form.baud ??= 115200;
      loadPorts();
    }
  }

  let ports = $state<{ path: string; description: string }[]>([]);
  async function loadPorts() {
    ports = await api.serialPorts().catch(() => []);
  }
  if (initial.kind === "serial") loadPorts();

  // Atlama sunucusu: kayıtlı bir oturum ya da elle yazılan "kullanıcı@sunucu:port".
  const jumpCandidates = $derived(store.sessions.filter((s) => s.id !== form.id && (s.kind ?? "ssh") === "ssh"));
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
      error = form.kind === "serial" ? "Seri port aygıtı seçin." : "Sunucu adresi gerekli.";
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
          jump: !isSsh && !isDesktop ? null : jumpMode === "custom" ? jumpCustom.trim() || null : jumpMode || null,
          x11: form.kind === "ssh" ? !!form.x11 : false,
          baud: form.kind === "serial" ? Number(form.baud) || 115200 : null,
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

<Modal title={session ? "Oturumu düzenle" : "Yeni oturum"} width={560} {onClose}>
  <form
    class="grid"
    onsubmit={(e) => {
      e.preventDefault();
      save(true);
    }}
  >
    <div class="kind full" role="radiogroup" aria-label="Bağlantı türü">
      {#each kinds as k}
        <button
          type="button"
          role="radio"
          aria-checked={(form.kind ?? "ssh") === k.key}
          class:on={(form.kind ?? "ssh") === k.key}
          onclick={() => setKind(k.key)}
        >
          <strong>{k.title}</strong>
          <span>{k.desc}</span>
        </button>
      {/each}
    </div>
    {#if form.kind === "serial"}
      <label class="wide">
        <span>Aygıt</span>
        <div class="row">
          <input bind:value={form.host} list="serial-ports" placeholder={navigator.platform.startsWith("Win") ? "COM3" : "/dev/ttyUSB0"} spellcheck="false" />
          <button type="button" class="btn" title="Aygıtları yeniden tara" onclick={loadPorts}>Tara</button>
        </div>
        <datalist id="serial-ports">
          {#each ports as p}<option value={p.path}>{p.description}</option>{/each}
        </datalist>
        <small>{ports.length ? `${ports.length} aygıt bulundu — listeden seçebilirsiniz` : "Aygıt bulunamadı; kabloyu takıp Tara'ya basın"}</small>
      </label>
      <label class="narrow">
        <span>Baud</span>
        <select bind:value={form.baud}>
          {#each bauds as b}<option value={b}>{b}</option>{/each}
        </select>
      </label>
    {:else}
      <label class="wide">
        <span>Sunucu</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input bind:value={form.host} placeholder="ornek.com veya 192.168.1.10" autofocus spellcheck="false" />
      </label>
      <label class="narrow">
        <span>Port</span>
        <input type="number" min="1" max="65535" bind:value={form.port} />
      </label>
    {/if}
    {#if isSsh}
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
    {#if form.kind === "ssh"}
      <label class="check full">
        <input type="checkbox" bind:checked={form.x11} />
        <span>X11 yönlendirme <small>Sunucudaki grafik uygulamalar bu bilgisayarda açılır (macOS'ta XQuartz, Windows'ta VcXsrv gerekir)</small></span>
      </label>
    {/if}
    {/if}
    {#if isDesktop}
      <label class="full">
        <span>Kullanıcı adı {form.kind === "vnc" ? "(bazı VNC sunucuları ister)" : ""}</span>
        <input bind:value={form.username} placeholder="isteğe bağlı" spellcheck="false" />
      </label>
      {#if form.kind === "vnc"}
        <label class="full">
          <span>VNC parolası</span>
          <input
            type="password"
            bind:value={secret}
            disabled={forget}
            placeholder={session?.hasSecret ? "•••••• (kayıtlı, değiştirmek için yazın)" : "boş bırakılırsa bağlanırken sorulur"}
          />
          <small>Parola işletim sisteminin kasasında tutulur.</small>
        </label>
        {#if session?.hasSecret}
          <label class="check full"><input type="checkbox" bind:checked={forget} /> Kayıtlı parolayı sil</label>
        {/if}
      {/if}
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
    {#if isSsh || isDesktop}
    <label class="full">
      <span>{isDesktop ? "SSH tüneli üzerinden bağlan" : "Atlama sunucusu (ProxyJump)"}</span>
      <select bind:value={jumpMode}>
        <option value="">{isDesktop ? "Hayır, doğrudan bağlan" : "Yok, doğrudan bağlan"}</option>
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
    {#if isDesktop && jumpMode}
      <small class="full hint">Sunucu adresi, seçilen SSH sunucusunun gözünden yazılır (ör. <code>localhost</code> ya da iç ağ adresi).</small>
    {/if}
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
    grid-template-columns: repeat(3, 1fr);
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
  .hint {
    color: var(--muted);
    font-size: 11.5px;
    margin-top: -6px;
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: 13px;
  }
</style>
