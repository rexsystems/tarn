<script lang="ts">
  import { sizedCover, srcsetFor, bannerSrcset, ytThumb } from '../cover';

  let {
    src = null,
    alt = '',
    size = 152,
    width = 0,
    height = 0,
    round = false,
    banner = false,
    /** YouTube video id - last-resort thumb when CDN art 404s. */
    videoId = null,
    class: cls = '',
  }: {
    src?: string | null;
    alt?: string;
    size?: number;
    width?: number;
    height?: number;
    round?: boolean;
    banner?: boolean;
    videoId?: string | null;
    class?: string;
  } = $props();

  let w = $derived(width || size);
  let h = $derived(height || size);
  let srcUrl = $derived(src ? sizedCover(src, Math.round(w * 2), Math.round(h * 2)) : '');
  let srcset = $derived(
    src ? (banner ? bannerSrcset(src, w, h) : srcsetFor(src, w, w === h)) : '',
  );

  let broken = $state(false);
  let useOrig = $state(false);
  let fallback = $state<string | null>(null);

  $effect(() => {
    void src;
    broken = false;
    useOrig = false;
    fallback = null;
  });

  let shown = $derived(fallback || (useOrig && src ? src : srcUrl));
  let shownSet = $derived(fallback || useOrig ? '' : srcset);
  let empty = $derived(!src);

  function onErr() {
    // Rewritten size failed - try the original URL from the API.
    if (!useOrig && !fallback && src && shown !== src) {
      useOrig = true;
      return;
    }
    // Some videos have no hq/maxres - step down the i.ytimg quality ladder.
    const ytimg = fallback || src;
    if (ytimg?.includes('i.ytimg.com')) {
      const lower = ytimg
        .replace(/\/(maxresdefault|hq720|sddefault|hqdefault)\.jpg/, '/mqdefault.jpg')
        .replace(/\/mqdefault\.jpg/, '/default.jpg');
      if (lower !== shown) {
        fallback = lower;
        return;
      }
    }
    // CDN / mixart 404 - public video thumb still loads without cookies.
    if (videoId && /^[\w-]{11}$/.test(videoId)) {
      const thumb = ytThumb(videoId);
      if (shown !== thumb && fallback !== thumb) {
        fallback = thumb;
        return;
      }
    }
    broken = true;
  }
</script>

{#if shown && !broken}
  <img
    class={cls}
    class:round
    {alt}
    width={w}
    height={h}
    src={shown}
    srcset={shownSet}
    sizes="{w}px"
    referrerpolicy="no-referrer"
    draggable="false"
    onerror={onErr}
  />
{:else}
  <div
    class="ph {cls}"
    class:round
    class:ask={broken || empty}
    style="width:{w}px;height:{h}px"
    aria-hidden="true"
  >{#if broken || empty}?{/if}</div>
{/if}

<style>
  .ph.ask {
    display: grid;
    place-items: center;
    color: var(--text-3, #6b7280);
    font-size: clamp(12px, 40%, 28px);
    font-weight: 600;
    line-height: 1;
    user-select: none;
  }
</style>
