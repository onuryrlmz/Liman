<script lang="ts" module>
  export interface MenuItem {
    label: string;
    icon?: string;
    danger?: boolean;
    disabled?: boolean;
    action: () => void;
  }
</script>

<script lang="ts">
  import Icon from "./Icon.svelte";

  let {
    x,
    y,
    items,
    onClose,
  }: { x: number; y: number; items: (MenuItem | null)[]; onClose: () => void } = $props();

  let menu: HTMLDivElement;
  let pos = $state({ left: 0, top: 0 });

  $effect(() => {
    const w = menu.offsetWidth;
    const h = menu.offsetHeight;
    pos = {
      left: Math.min(x, window.innerWidth - w - 4),
      top: Math.min(y, window.innerHeight - h - 4),
    };
  });
</script>

<svelte:window
  onkeydown={(e) => e.key === "Escape" && onClose()}
  onblur={onClose}
/>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div
  class="backdrop"
  onclick={onClose}
  oncontextmenu={(e) => {
    e.preventDefault();
    onClose();
  }}
></div>
<div class="menu" bind:this={menu} style:left="{pos.left}px" style:top="{pos.top}px" role="menu">
  {#each items as item}
    {#if item === null}
      <div class="sep"></div>
    {:else}
      <button
        role="menuitem"
        class:danger={item.danger}
        disabled={item.disabled}
        onclick={() => {
          onClose();
          item.action();
        }}
      >
        <span class="ic">{#if item.icon}<Icon name={item.icon} size={14} />{/if}</span>
        {item.label}
      </button>
    {/if}
  {/each}
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 90;
  }
  .menu {
    position: fixed;
    z-index: 91;
    min-width: 180px;
    padding: 4px;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.45);
  }
  button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--text);
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  button:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  button.danger {
    color: var(--danger);
  }
  .ic {
    width: 14px;
    display: inline-flex;
    color: var(--muted);
  }
  .danger .ic {
    color: inherit;
  }
  .sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
</style>
