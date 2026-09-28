<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import Modal from "./Modal.svelte";
  import { api } from "./api";
  import Updater from "./Updater.svelte";
  import { settings, themes, fonts, defaults, MIN_FONT, MAX_FONT } from "./settings.svelte";

  let { onClose }: { onClose: () => void } = $props();

  const s = $derived(settings.value);
  const fontLabel = (f: string) => f.split(",")[0].replaceAll('"', "");
  let defaultLogDir = $state("");
  api.logDefaultDir().then((d) => (defaultLogDir = d)).catch(() => {});
  const logDir = $derived(s.logDir || defaultLogDir);

  async function pickLogDir() {
    const d = await open({ directory: true, title: "Kayıt klasörü", defaultPath: logDir || undefined });
    if (typeof d === "string") settings.update({ logDir: d });
  }

  const mod = navigator.platform.toLowerCase().includes("mac") ? "⌘" : "Ctrl";
</script>

<Modal title="Ayarlar" width={520} {onClose}>
  <div class="sections">
    <section>
      <h3>Terminal görünümü</h3>
      <div class="grid">
        <label>
          <span>Tema</span>
          <select value={s.theme} onchange={(e) => settings.update({ theme: e.currentTarget.value })}>
            {#each Object.entries(themes) as [key, t]}<option value={key}>{t.label}</option>{/each}
          </select>
        </label>
        <label>
          <span>Yazı tipi</span>
          <select value={s.fontFamily} onchange={(e) => settings.update({ fontFamily: e.currentTarget.value })}>
            {#each fonts as f}<option value={f}>{fontLabel(f)}</option>{/each}
          </select>
        </label>
        <label>
          <span>Yazı boyutu ({mod} + / − / 0)</span>
          <div class="row">
            <input
              type="range"
              min={MIN_FONT}
              max={MAX_FONT}
              value={s.fontSize}
              oninput={(e) => settings.update({ fontSize: Number(e.currentTarget.value) })}
            />
            <span class="val">{s.fontSize}</span>
          </div>
        </label>
        <label>
          <span>Satır aralığı</span>
          <div class="row">
            <input
              type="range"
              min="1"
              max="1.6"
              step="0.05"
              value={s.lineHeight}
              oninput={(e) => settings.update({ lineHeight: Number(e.currentTarget.value) })}
            />
            <span class="val">{s.lineHeight.toFixed(2)}</span>
          </div>
        </label>
        <label>
          <span>İmleç</span>
          <select value={s.cursorStyle} onchange={(e) => settings.update({ cursorStyle: e.currentTarget.value as typeof s.cursorStyle })}>
            <option value="block">Blok</option>
            <option value="bar">Çizgi</option>
            <option value="underline">Alt çizgi</option>
          </select>
        </label>
        <label>
          <span>Geçmiş (satır)</span>
          <input
            type="number"
            min="1000"
            max="200000"
            step="1000"
            value={s.scrollback}
            onchange={(e) => settings.update({ scrollback: Math.max(1000, Number(e.currentTarget.value) || defaults.scrollback) })}
          />
        </label>
      </div>
      <label class="check"><input type="checkbox" checked={s.cursorBlink} onchange={(e) => settings.update({ cursorBlink: e.currentTarget.checked })} /> İmleç yanıp sönsün</label>
    </section>

    <section>
      <h3>Davranış</h3>
      <label class="check"><input type="checkbox" checked={s.copyOnSelect} onchange={(e) => settings.update({ copyOnSelect: e.currentTarget.checked })} /> Seçilen metni otomatik kopyala</label>
      <label class="check"><input type="checkbox" checked={s.rightClickPaste} onchange={(e) => settings.update({ rightClickPaste: e.currentTarget.checked })} /> Sağ tıkla yapıştır</label>
      <label class="check">
        <input type="checkbox" checked={s.autoReconnect} onchange={(e) => settings.update({ autoReconnect: e.currentTarget.checked })} />
        <span>Kopan SSH bağlantılarına otomatik yeniden bağlan <small>Ağ değişince ya da bilgisayar uykudan uyanınca</small></span>
      </label>
    </section>

    <section>
      <h3>Oturum kaydı</h3>
      <label class="check">
        <input type="checkbox" checked={s.sessionLog} onchange={(e) => settings.update({ sessionLog: e.currentTarget.checked })} />
        <span>Terminal çıktısını dosyaya kaydet <small>Her yeni terminal için ayrı, renk kodlarından arındırılmış bir .log dosyası</small></span>
      </label>
      {#if s.sessionLog}
        <div class="logdir">
          <code title={logDir}>{logDir}</code>
          <button class="btn" onclick={pickLogDir}>Değiştir…</button>
          <button class="btn" onclick={() => revealItemInDir(logDir).catch(() => {})}>Aç</button>
        </div>
        <label class="check"><input type="checkbox" checked={s.logTimestamps} onchange={(e) => settings.update({ logTimestamps: e.currentTarget.checked })} /> Satırlara saat ekle</label>
        <p class="hint">Kayıt, bu ayar açıkken başlatılan terminallerde yapılır. Parolalar ekrana yazılmadığı için kayda da girmez.</p>
      {/if}
    </section>

    <section>
      <h3>Güncellemeler</h3>
      <label class="check">
        <input type="checkbox" checked={s.checkUpdates} onchange={(e) => settings.update({ checkUpdates: e.currentTarget.checked })} />
        Açılışta yeni sürümü denetle
      </label>
      <Updater />
    </section>
  </div>

  {#snippet footer()}
    <button class="btn" onclick={() => settings.update({ ...defaults, checkUpdates: s.checkUpdates })}>Varsayılanlara dön</button>
    <button class="btn primary" onclick={onClose}>Tamam</button>
  {/snippet}
</Modal>

<style>
  .sections {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: 11.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.6px;
    color: var(--muted);
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
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
    align-items: flex-start;
    gap: 9px;
    color: var(--text);
    font-size: 13px;
  }
  label.check input {
    margin-top: 2px;
  }
  small {
    display: block;
    color: var(--muted);
    font-size: 11.5px;
  }
  .logdir {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .logdir code {
    flex: 1;
    min-width: 0;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  .logdir .btn {
    padding: 5px 10px;
  }
  .hint {
    margin: 0;
    font-size: 11.5px;
    color: var(--muted);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .row input {
    flex: 1;
    padding: 0;
    accent-color: var(--accent);
  }
  .val {
    width: 32px;
    text-align: right;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
</style>
