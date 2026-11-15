<script lang="ts">
  import Cover from './Cover.svelte';
  import Hi from './Hi.svelte';
  import { sizedCover } from '../cover';
  import { ThumbsUpIcon, Bookmark02Icon } from '../icons';

  let {
    id = '',
    name = '',
    cover = null,
    covers = [],
    size = 152,
    alt = '',
    class: cls = '',
  }: {
    id?: string;
    name?: string;
    cover?: string | null;
    covers?: string[] | null;
    size?: number;
    alt?: string;
    class?: string;
  } = $props();

  let kind = $derived((() => {
    const pid = (id || '').replace(/^VL/, '');
    const n = (name || '').toLowerCase();
    // Home Fresh finds / Listen again use RDAMPLLM, library uses LM.
    if (pid === 'LM' || pid === 'RDAMPLLM' || n === 'liked music' || n === 'liked songs') {
      return 'liked' as const;
    }
    if (pid === 'SE' || n.includes('episodes for later')) return 'episodes' as const;
    return 'normal' as const;
  })());

  function coverKey(u: string): string {
    const vi = u.split('/vi/')[1];
    if (vi) {
      const id = vi.split('/')[0];
      if (id && id.length === 11) return id;
    }
    const base = u.split('?')[0] || u;
    const eq = base.lastIndexOf('=');
    if (
      eq > 0 &&
      (base.includes('googleusercontent') || base.includes('ggpht') || base.includes('yt3.'))
    ) {
      return base.slice(0, eq);
    }
    return base;
  }

  let tiles = $derived((() => {
    const out: string[] = [];
    const seen = new Set<string>();
    for (const u of covers ?? []) {
      if (!u || u.includes('/mixart') || u.includes('music.youtube.com/image/')) continue;
      const k = coverKey(u);
      if (seen.has(k)) continue;
      seen.add(k);
      out.push(u);
      if (out.length === 4) break;
    }
    return out;
  })());
  // Full 2×2 only — 2–3 tiles used to leave black placeholder cells on Home.
  let mosaic = $derived(kind === 'normal' && tiles.length >= 4);
  let iconSize = $derived(Math.max(14, Math.round(size * 0.42)));
</script>

{#if kind === 'liked'}
  <div
    class="special liked {cls}"
    style="width:{size}px;height:{size}px"
    role={alt ? 'img' : undefined}
    aria-label={alt || 'Liked Music'}
    aria-hidden={alt ? undefined : true}
  >
    <Hi icon={ThumbsUpIcon} size={iconSize} />
  </div>
{:else if kind === 'episodes'}
  <div
    class="special episodes {cls}"
    style="width:{size}px;height:{size}px"
    role={alt ? 'img' : undefined}
    aria-label={alt || 'Episodes for later'}
    aria-hidden={alt ? undefined : true}
  >
    <Hi icon={Bookmark02Icon} size={iconSize} />
  </div>
{:else if mosaic}
  <div
    class="mosaic {cls}"
    style="width:{size}px;height:{size}px"
    aria-hidden={alt ? undefined : true}
    role={alt ? 'img' : undefined}
    aria-label={alt || undefined}
  >
    {#each tiles.slice(0, 4) as src, i (src + i)}
      <img
        src={sizedCover(src, Math.max(48, Math.round(size)))}
        {alt}
        referrerpolicy="no-referrer"
        draggable="false"
      />
    {/each}
  </div>
{:else}
  <Cover src={cover || tiles[0] || null} {size} {alt} class={cls} />
{/if}

<style>
  .mosaic {
    display: grid;
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
    gap: 1px;
    border-radius: 6px;
    overflow: hidden;
    background: var(--bg-elev);
    flex-shrink: 0;
  }
  .mosaic img,
  .mosaic .ph {
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    object-fit: cover;
    background: var(--bg-elev);
    display: block;
  }
  .special {
    display: grid;
    place-items: center;
    border-radius: 6px;
    flex-shrink: 0;
    color: #fff;
  }
  .special.liked {
    background: linear-gradient(135deg, #4a148c 0%, #c2185b 55%, #ec407a 100%);
  }
  .special.episodes {
    background: linear-gradient(135deg, #0d47a1 0%, #00838f 100%);
  }
</style>
