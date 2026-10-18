<script lang="ts">
  import { currentTheme, setTheme, THEMES } from '../theme';
  import Icon from './Icon.svelte';

  /** The header's theme button and its menu. */
  let open = $state(false);
  let theme = $state(currentTheme());
  let root: HTMLDivElement | undefined = $state();

  function choose(id: string) {
    theme = id;
    setTheme(id);
    open = false;
  }

  // Close when clicking elsewhere.
  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) open = false;
    };
    document.addEventListener('pointerdown', onDown);
    return () => document.removeEventListener('pointerdown', onDown);
  });
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && open && (open = false)} />

<div class="theme-menu" bind:this={root}>
  <button class="icon" onclick={() => ((theme = currentTheme()), (open = !open))} aria-haspopup="menu" aria-expanded={open} aria-label="Theme" title="Theme">
    <Icon name="palette" />
  </button>
  {#if open}
    <div class="menu" role="menu" aria-label="Themes">
      {#each THEMES as t (t.id)}
        <button class="item" role="menuitemradio" aria-checked={t.id === theme} onclick={() => choose(t.id)}>
          <span class="swatch" style="background: {t.swatch[0]}; border-color: {t.swatch[1]}">
            <span style="background: {t.swatch[1]}"></span>
            <span style="background: {t.swatch[2]}"></span>
          </span>
          <span class="name">{t.name}</span>
          {#if t.id === theme}<Icon name="check" size={16} />{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .theme-menu {
    position: relative;
  }

  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    z-index: 50;
    min-width: 270px;
    padding: 6px;
    display: grid;
    border-radius: 12px;
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.35);
  }

  .item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 10px;
    border: 0;
    border-radius: 8px;
    background: none;
    text-align: left;
    color: var(--text);
  }

  .item:hover,
  .item:focus-visible {
    background: var(--surface-2);
  }

  .name {
    flex: 1;
    white-space: nowrap;
  }

  .swatch {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 3px;
    width: 34px;
    height: 22px;
    border-radius: 6px;
    border: 1px solid;
  }

  .swatch span {
    width: 9px;
    height: 9px;
    border-radius: 50%;
  }
</style>
