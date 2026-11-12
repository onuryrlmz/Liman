<script lang="ts">
  import Modal from "./Modal.svelte";
  import { api, errText, type Entry } from "./api";
  import { store } from "./tabs.svelte";
  import { WHO, WHAT, SPECIAL, parseOctal, toOctal, toggle, startMode, type ChmodScope } from "./perms";

  let {
    termId,
    entries,
    onClose,
    onDone,
  }: { termId: string; entries: Entry[]; onClose: () => void; onDone: () => void } = $props();

  // Açılıştaki seçim sabit kalır; panel arkada yenilense de pencere aynı öğelere uygular.
  const targets = entries.slice();
  const start = startMode(targets.map((e) => e.mode));
  const hasDir = targets.some((e) => e.isDir && !e.isLink);

  let mode = $state(start.mode);
  let text = $state(toOctal(start.mode));
  let scope = $state<ChmodScope>("item");
  let busy = $state(false);
  let error = $state("");

  const valid = $derived(parseOctal(text) !== null);
  const title = targets.length === 1 ? `İzinler: ${targets[0].name}` : `İzinler: ${targets.length} öğe`;

  function setBit(bit: number, on: boolean) {
    mode = toggle(mode, bit, on);
    text = toOctal(mode);
  }

  function onText() {
    const m = parseOctal(text);
    if (m !== null) mode = m;
  }

  async function apply() {
    const m = parseOctal(text);
    if (m === null || busy) return;
    busy = true;
    error = "";
    let changed = 0;
    const failed: string[] = [];
    let failedCount = 0;
    for (const e of targets) {
      try {
        const r = await api.sftpChmod(termId, e.path, m, e.isDir ? scope : "item");
        changed += r.changed;
        failedCount += r.failedCount;
        for (const [p, msg] of r.failed) if (failed.length < 5) failed.push(`${p}: ${msg}`);
      } catch (err) {
        failedCount++;
        if (failed.length < 5) failed.push(`${e.name}: ${errText(err)}`);
      }
    }
    busy = false;
    onDone();
    if (failedCount) {
      error = `${failedCount} öğenin izni değiştirilemedi${changed ? ` (${changed} öğe değişti)` : ""}:\n${failed.join("\n")}`;
      return;
    }
    store.notify(changed === 1 ? `İzin ${toOctal(m)} yapıldı` : `${changed} öğenin izni ${toOctal(m)} yapıldı`);
    onClose();
  }
</script>

<Modal {title} width={420} {onClose}>
  <form
    class="form"
    onsubmit={(e) => {
      e.preventDefault();
      apply();
    }}
  >
    <table>
      <thead>
        <tr>
          <th></th>
          {#each WHAT as w}<th>{w.label}</th>{/each}
        </tr>
      </thead>
      <tbody>
        {#each WHO as who}
          <tr>
            <th scope="row">{who.label}</th>
            {#each WHAT as w}
              {@const bit = w.bit << who.shift}
              <td>
                <input
                  type="checkbox"
                  aria-label="{who.label} {w.label}"
                  checked={(mode & bit) !== 0}
                  onchange={(e) => setBit(bit, e.currentTarget.checked)}
                />
              </td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>

    <div class="special">
      {#each SPECIAL as s}
        <label class="check small">
          <input type="checkbox" checked={(mode & s.bit) !== 0} onchange={(e) => setBit(s.bit, e.currentTarget.checked)} />
          {s.label}
        </label>
      {/each}
    </div>

    <label>
      <span>Sayısal değer</span>
      <input class="octal" bind:value={text} oninput={onText} maxlength="4" spellcheck="false" aria-invalid={!valid} />
      {#if !valid}<small class="err">3 ya da 4 sekizli rakam yazın (ör. 755, 644)</small>{/if}
      {#if start.mixed}<small>Seçilenlerin izinleri farklı; uygulayınca hepsi bu değer olur.</small>{/if}
    </label>

    {#if hasDir}
      <label>
        <span>Klasörlerde</span>
        <select bind:value={scope}>
          <option value="item">Yalnızca klasörün kendisine uygula</option>
          <option value="all">Alt klasör ve dosyalara da uygula (-R)</option>
          <option value="files">Yalnızca içindeki dosyalara uygula</option>
          <option value="dirs">Yalnızca klasöre ve alt klasörlere uygula</option>
        </select>
        {#if scope !== "item"}<small>Sembolik bağlantılara ve içlerine dokunulmaz.</small>{/if}
      </label>
    {/if}

    {#if error}<p class="err box">{error}</p>{/if}
    <button type="submit" hidden aria-hidden="true"></button>
  </form>

  {#snippet footer()}
    <button class="btn" onclick={onClose}>{error ? "Kapat" : "Vazgeç"}</button>
    <button class="btn primary" disabled={!valid || busy} onclick={apply}>{busy ? "Uygulanıyor…" : "Uygula"}</button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  table {
    border-collapse: collapse;
    font-size: 12.5px;
  }
  th {
    font-weight: 500;
    color: var(--muted);
    padding: 4px 10px;
    text-align: center;
  }
  th[scope="row"] {
    text-align: left;
    color: var(--text);
    padding-left: 0;
  }
  td {
    text-align: center;
    padding: 4px 10px;
  }
  .special {
    display: flex;
    gap: 16px;
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
    align-items: center;
    gap: 6px;
    color: var(--text);
  }
  .octal {
    width: 90px;
    font-family: "JetBrains Mono", Menlo, Consolas, monospace;
  }
  small {
    font-size: 11.5px;
  }
  .err {
    color: var(--danger);
  }
  .box {
    margin: 0;
    white-space: pre-wrap;
    font-size: 12px;
    word-break: break-word;
  }
</style>
