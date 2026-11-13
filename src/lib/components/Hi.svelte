<script lang="ts">
  import { HugeiconsIcon } from '@hugeicons/svelte';
  import type { IconSvgElement } from '@hugeicons/svelte';

  let {
    icon,
    altIcon = undefined,
    showAlt = false,
    size = 22,
    strokeWidth = 1.75,
    color = 'currentColor',
    class: cls = '',
  }: {
    icon: IconSvgElement;
    altIcon?: IconSvgElement;
    showAlt?: boolean;
    size?: number;
    strokeWidth?: number;
    color?: string;
    class?: string;
  } = $props();
</script>

<!-- HugeiconsIcon only applies `icon` on mount; key forces a fresh glyph when it changes. -->
{#key showAlt ? 'alt' : icon}
  <HugeiconsIcon {icon} {altIcon} {showAlt} {size} {strokeWidth} {color} class={cls} absoluteStrokeWidth />
{/key}
