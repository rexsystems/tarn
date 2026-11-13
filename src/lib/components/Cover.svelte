<script lang="ts">
  import { sizedCover, srcsetFor, bannerSrcset } from '../cover';

  let {
    src = null,
    alt = '',
    size = 152,
    width = 0,
    height = 0,
    round = false,
    banner = false,
    class: cls = '',
  }: {
    src?: string | null;
    alt?: string;
    size?: number;
    width?: number;
    height?: number;
    round?: boolean;
    banner?: boolean;
    class?: string;
  } = $props();

  let w = $derived(width || size);
  let h = $derived(height || size);
  let srcUrl = $derived(src ? sizedCover(src, Math.round(w * 2), Math.round(h * 2)) : '');
  let srcset = $derived(
    src ? (banner ? bannerSrcset(src, w, h) : srcsetFor(src, w, w === h)) : '',
  );
</script>

{#if srcUrl}
  <img
    class={cls}
    class:round
    {alt}
    width={w}
    height={h}
    src={srcUrl}
    srcset={srcset}
    sizes="{w}px"
    draggable="false"
  />
{:else}
  <div class="ph {cls}" class:round style="width:{w}px;height:{h}px"></div>
{/if}
