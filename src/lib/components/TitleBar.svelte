<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onMount } from 'svelte';
  import Hi from './Hi.svelte';
  import { ArrowLeft01Icon, ArrowRight01Icon } from '../icons';

  let {
    canBack = false,
    canFwd = false,
    onBack,
    onFwd,
  }: {
    canBack?: boolean;
    canFwd?: boolean;
    onBack?: () => void;
    onFwd?: () => void;
  } = $props();

  const isMac =
    typeof navigator !== 'undefined' &&
    (/Mac/i.test(navigator.platform) || /macOS|Macintosh/i.test(navigator.userAgent));

  let maximized = $state(false);
  const win = typeof window !== 'undefined' ? getCurrentWindow() : null;

  onMount(() => {
    if (!win) return;
    const sync = () => win.isMaximized().then((v) => (maximized = v)).catch(() => {});
    sync();
    const unsubs: Array<() => void> = [];
    win.onResized(sync).then((u) => unsubs.push(u)).catch(() => {});
    return () => unsubs.forEach((u) => u());
  });

  async function minimize() {
    await win?.minimize();
  }
  async function toggleMax() {
    await win?.toggleMaximize();
    maximized = (await win?.isMaximized()) ?? !maximized;
  }
  async function close() {
    await win?.close();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- Drag only on title/spacer - whole-header drag-region breaks opacity fades on Windows. -->
<header
  class="titlebar {isMac ? 'mac' : 'win'}"
  ondblclick={() => !isMac && toggleMax()}
>
  {#if isMac}
    <div class="traffic-space" data-tauri-drag-region></div>
  {/if}
  <div class="tb-nav">
    <button
      class="tb-nav-btn"
      type="button"
      title="Back"
      aria-label="Back"
      disabled={!canBack}
      onclick={() => onBack?.()}
    >
      <Hi icon={ArrowLeft01Icon} size={16} />
    </button>
    <button
      class="tb-nav-btn"
      type="button"
      title="Forward"
      aria-label="Forward"
      disabled={!canFwd}
      onclick={() => onFwd?.()}
    >
      <Hi icon={ArrowRight01Icon} size={16} />
    </button>
  </div>
  <div class="tb-title" data-tauri-drag-region>Tarn</div>
  <div class="tb-drag" data-tauri-drag-region></div>
  {#if !isMac}
    <div class="tb-btns">
      <button class="tb-btn min" type="button" title="Minimize" onclick={minimize} aria-label="Minimize">
        <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><path d="M1.5 6h9" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" fill="none"/></svg>
      </button>
      <button class="tb-btn max" type="button" title={maximized ? 'Restore' : 'Maximize'} onclick={toggleMax} aria-label={maximized ? 'Restore' : 'Maximize'}>
        {#if maximized}
          <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
            <path d="M3.5 3h6v6H8.5" stroke="currentColor" stroke-width="1.35" stroke-linejoin="round" fill="none"/>
            <rect x="1.5" y="4" width="6.5" height="6.5" stroke="currentColor" stroke-width="1.35" fill="none"/>
          </svg>
        {:else}
          <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
            <rect x="2" y="2" width="8" height="8" stroke="currentColor" stroke-width="1.35" fill="none"/>
          </svg>
        {/if}
      </button>
      <button class="tb-btn close" type="button" title="Close" onclick={close} aria-label="Close">
        <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
          <path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" fill="none"/>
        </svg>
      </button>
    </div>
  {/if}
</header>
