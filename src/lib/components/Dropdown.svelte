<script lang="ts">
  import { onMount } from 'svelte';

  type Opt = { value: string; label: string };
  let {
    value = $bindable(),
    options,
    onchange,
    disabled = false,
    align = 'right',
  }: {
    value: string;
    options: Opt[];
    onchange?: (v: string) => void;
    disabled?: boolean;
    align?: 'left' | 'right';
  } = $props();

  let open = $state(false);
  let root: HTMLDivElement | undefined = $state();
  let active = $state(-1);
  const label = $derived(options.find((o) => o.value === value)?.label ?? value);

  function close() {
    open = false;
    active = -1;
  }
  function pick(v: string) {
    value = v;
    close();
    onchange?.(v);
  }
  function toggle() {
    if (disabled) return;
    open = !open;
    if (open) active = Math.max(0, options.findIndex((o) => o.value === value));
  }
  function onDoc(e: PointerEvent) {
    if (root && !root.contains(e.target as Node)) close();
  }
  function onKey(e: KeyboardEvent) {
    if (disabled) return;
    if (!open) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        open = true;
        active = Math.max(0, options.findIndex((o) => o.value === value));
      }
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
      (root?.querySelector('.dd-btn') as HTMLButtonElement | null)?.focus();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      active = (active + 1) % options.length;
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      active = (active - 1 + options.length) % options.length;
    } else if (e.key === 'Home') {
      e.preventDefault();
      active = 0;
    } else if (e.key === 'End') {
      e.preventDefault();
      active = options.length - 1;
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      if (active >= 0 && options[active]) pick(options[active].value);
    }
  }

  $effect(() => {
    if (!open) return;
    document.addEventListener('pointerdown', onDoc, true);
    return () => document.removeEventListener('pointerdown', onDoc, true);
  });

  $effect(() => {
    if (!open || active < 0) return;
    const el = root?.querySelector(`[data-i="${active}"]`) as HTMLElement | null;
    el?.scrollIntoView({ block: 'nearest' });
  });

  onMount(() => () => close());
</script>

<div class="dd {open ? 'open' : ''}" bind:this={root}>
  <button
    class="dd-btn"
    type="button"
    {disabled}
    aria-haspopup="listbox"
    aria-expanded={open}
    onclick={toggle}
    onkeydown={onKey}
  >
    <span class="dd-val">{label}</span>
    <svg class="dd-caret" width="14" height="14" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
    </svg>
  </button>
  {#if open}
    <ul class="dd-menu {align}" role="listbox" tabindex="-1" aria-activedescendant={active >= 0 ? `dd-${options[active]?.value}` : undefined}>
      {#each options as o, i}
        <li role="presentation">
          <button
            id="dd-{o.value}"
            class="dd-opt {o.value === value ? 'sel' : ''} {i === active ? 'act' : ''}"
            type="button"
            role="option"
            data-i={i}
            aria-selected={o.value === value}
            onpointerenter={() => (active = i)}
            onclick={() => pick(o.value)}
          >
            {o.label}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>
