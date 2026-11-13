<script lang="ts">
  import Hi from './Hi.svelte';
  import type { IconSvgElement } from '@hugeicons/svelte';

  let {
    icon,
    altIcon = undefined,
    showAlt = false,
    size = 22,
    btnSize = 'md',
    on = false,
    danger = false,
    disabled = false,
    title = '',
    spinning = false,
    onclick,
    class: cls = '',
  }: {
    icon: IconSvgElement;
    altIcon?: IconSvgElement;
    showAlt?: boolean;
    size?: number;
    btnSize?: 'md' | 'lg' | 'xl';
    on?: boolean;
    danger?: boolean;
    disabled?: boolean;
    title?: string;
    onclick?: (e: MouseEvent) => void;
    class?: string;
    spinning?: boolean;
  } = $props();
</script>

<button
  class="ibtn {btnSize === 'lg' ? 'lg' : ''} {btnSize === 'xl' ? 'xl' : ''} {on ? 'on' : ''} {danger ? 'danger' : ''} {cls}"
  class:spinning
  {title}
  {disabled}
  {onclick}
  type="button"
>
  {#if spinning}
    <span class="ibtn-spin" aria-hidden="true"></span>
  {:else}
    <Hi {icon} {altIcon} {showAlt} {size} />
  {/if}
</button>
