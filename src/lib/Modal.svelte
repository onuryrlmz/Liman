<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  let {
    title,
    width = 460,
    onClose,
    children,
    footer,
  }: {
    title: string;
    width?: number;
    onClose: () => void;
    children: Snippet;
    footer?: Snippet;
  } = $props();
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onClose()} />

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="overlay" onmousedown={(e) => e.target === e.currentTarget && onClose()}>
  <div class="modal" style:width="min({width}px, calc(100vw - 32px))" role="dialog" aria-label={title}>
    <header>
      <h2>{title}</h2>
      <button class="icon-btn" onclick={onClose} aria-label="Kapat"><Icon name="x" /></button>
    </header>
    <div class="body">{@render children()}</div>
    {#if footer}<footer>{@render footer()}</footer>{/if}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: grid;
    place-items: center;
    background: rgb(5 8 12 / 0.6);
  }
  .modal {
    max-height: calc(100vh - 48px);
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 20px 60px rgb(0 0 0 / 0.5);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px 8px 18px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .body {
    padding: 6px 18px 16px;
    overflow: auto;
    min-height: 0;
    flex: 1;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--border);
  }
</style>
