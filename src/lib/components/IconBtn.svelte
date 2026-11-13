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
  } = $props();
</script>

<button
  class="ibtn {btnSize === 'lg' ? 'lg' : ''} {btnSize === 'xl' ? 'xl' : ''} {on ? 'on' : ''} {danger ? 'danger' : ''} {cls}"
  {title}
  {disabled}
  {onclick}
  type="button"
>
  <Hi {icon} {altIcon} {showAlt} {size} />
</button>
