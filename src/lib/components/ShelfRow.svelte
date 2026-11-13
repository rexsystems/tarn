<script lang="ts">
  import type { Shelf, ShelfItem, Track } from '../types';
  import { itemToTrack } from '../cover';
  import Cover from './Cover.svelte';
  import Hi from './Hi.svelte';
  import { PlayIcon } from '../icons';

  let {
    shelf,
    onPlayItems,
    onOpen,
    onContext,
  }: {
    shelf: Shelf;
    onPlayItems: (tracks: Track[], index: number) => void;
    onOpen: (item: ShelfItem) => void;
    onContext: (e: MouseEvent, item: ShelfItem) => void;
  } = $props();

  function tracksOf(items: ShelfItem[]): Track[] {
    return items.filter((i) => i.kind === 'track' || i.kind === 'video').map(itemToTrack);
  }

  function click(item: ShelfItem, i: number) {
    if (item.kind === 'track' || item.kind === 'video') {
      const ts = tracksOf(shelf.items);
      const idx = ts.findIndex((t) => t.id === item.id);
      onPlayItems(ts.length ? ts : [itemToTrack(item)], Math.max(0, idx === -1 ? i : idx));
    } else {
      onOpen(item);
    }
  }

  let compact = $derived(
    (shelf.kind === 'tracks' || shelf.title.toLowerCase().includes('quick')) &&
      shelf.items.length > 6 &&
      shelf.items.every((i) => i.kind === 'track' || i.kind === 'video'),
  );
  let moods = $derived(shelf.kind === 'moods' || shelf.items.every((i) => i.kind === 'mood'));
</script>

<section class="shelf">
  <div class="shelf-head">
    <div>
      {#if shelf.subtitle}
        <div class="shelf-sub">{shelf.subtitle}</div>
      {/if}
      <h2 class="shelf-title">{shelf.title}</h2>
    </div>
  </div>

  {#if moods}
    <div class="mood-grid">
      {#each shelf.items as item}
        <button
          class="mood"
          style="background:{item.color || '#3a4a5c'}"
          onclick={() => onOpen(item)}
          oncontextmenu={(e) => onContext(e, item)}
        >
          {item.title}
        </button>
      {/each}
    </div>
  {:else if compact}
    <div class="qp">
      {#each shelf.items as item, i}
        <button
          class="qp-row"
          onclick={() => click(item, i)}
          oncontextmenu={(e) => onContext(e, item)}
        >
          <span class="cov qp-cov">
            <Cover src={item.cover} size={48} alt="" />
            {#if item.kind === 'track' || item.kind === 'video'}
              <span class="play-ov"><Hi icon={PlayIcon} size={18} /></span>
            {/if}
          </span>
          <div class="tx">
            <div class="title">{item.title}</div>
            <div class="sub">{item.subtitle || item.artists.join(', ')}</div>
          </div>
        </button>
      {/each}
    </div>
  {:else}
    <div class="row">
      {#each shelf.items as item, i}
        <button
          class="card {item.kind === 'artist' && shelf.kind !== 'albums' && shelf.kind !== 'playlists' && !shelf.title.toLowerCase().includes('album') ? 'artist' : ''}"
          onclick={() => click(item, i)}
          oncontextmenu={(e) => onContext(e, item)}
        >
          <div class="art">
            <Cover
              src={item.cover}
              size={item.kind === 'artist' && shelf.kind !== 'albums' && !shelf.title.toLowerCase().includes('album') ? 132 : 152}
              round={item.kind === 'artist' && shelf.kind !== 'albums' && !shelf.title.toLowerCase().includes('album')}
              alt=""
            />
            {#if item.kind === 'track' || item.kind === 'video'}
              <span class="play-ov"><Hi icon={PlayIcon} size={36} /></span>
            {/if}
          </div>
          <div class="c-a">
            <div class="title">{item.title}</div>
            <div class="sub">
              {item.subtitle || item.artists.join(', ') || item.album_type || item.year || ''}
            </div>
          </div>
        </button>
      {/each}
    </div>
  {/if}
</section>
