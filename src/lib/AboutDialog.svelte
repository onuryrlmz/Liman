<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import Modal from "./Modal.svelte";
  import Icon from "./Icon.svelte";

  let { onClose }: { onClose: () => void } = $props();

  let version = $state("");
  getVersion()
    .then((v) => (version = v))
    .catch(() => {});

  const features = [
    { icon: "terminal", text: "Sekmeli SSH ve yerel terminal" },
    { icon: "folder", text: "Bağlanınca açılan SFTP dosya tarayıcısı" },
    { icon: "server", text: "Klasörlü oturum yöneticisi" },
    { icon: "key", text: "Parolalar işletim sisteminin kasasında" },
    { icon: "tunnel", text: "Port yönlendirme (tüneller)" },
    { icon: "edit", text: "Uzak dosyaları yerinde düzenleme" },
  ];
</script>

<Modal title="Hakkında" width={440} {onClose}>
  <div class="about">
    <div class="logo"><Icon name="anchor" size={34} /></div>
    <h1>Liman</h1>
    {#if version}<span class="version">Sürüm {version}</span>{/if}
    <p class="tagline">SSH, SFTP ve terminal için tek liman.</p>
    <p class="desc">
      Sunucularınıza bağlandığınız, dosyalarınızı taşıdığınız ve tünellerinizi açtığınız her şey tek pencerede.
      macOS, Windows ve Linux'ta aynı şekilde çalışır.
    </p>

    <ul>
      {#each features as f}
        <li><Icon name={f.icon} size={14} /> {f.text}</li>
      {/each}
    </ul>

    <p class="stack">Tauri · Rust · Svelte · xterm.js · russh ile yapıldı</p>

    <div class="copyright">
      <strong>© 2026 onuryrlmz</strong>
      <span>Tüm hakları saklıdır.</span>
      <span class="by">by onuryrlmz ⚓</span>
    </div>
  </div>
</Modal>

<style>
  .about {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    padding-top: 4px;
    user-select: text;
  }
  .logo {
    display: grid;
    place-items: center;
    width: 68px;
    height: 68px;
    border-radius: 18px;
    background: linear-gradient(145deg, #8fe0c8, #4fb89a);
    color: #0b1714;
    box-shadow: 0 8px 24px rgb(127 209 185 / 0.25);
  }
  h1 {
    margin: 12px 0 2px;
    font-size: 24px;
    letter-spacing: 0.3px;
  }
  .version {
    font-size: 12px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .tagline {
    margin: 10px 0 4px;
    font-weight: 600;
    color: var(--accent);
  }
  .desc {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.55;
    color: var(--muted);
    max-width: 340px;
  }
  ul {
    list-style: none;
    margin: 16px 0 0;
    padding: 12px 16px;
    display: grid;
    gap: 7px;
    width: 100%;
    text-align: left;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 12.5px;
  }
  li {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  li :global(svg) {
    color: var(--accent);
    flex: none;
  }
  .stack {
    margin: 14px 0 0;
    font-size: 11.5px;
    color: var(--muted);
  }
  .copyright {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 16px;
    padding-top: 14px;
    width: 100%;
    border-top: 1px solid var(--border);
    font-size: 12px;
    color: var(--muted);
  }
  .copyright strong {
    color: var(--text);
    font-size: 13px;
  }
  .by {
    margin-top: 4px;
    color: var(--accent);
    font-weight: 600;
  }
</style>
