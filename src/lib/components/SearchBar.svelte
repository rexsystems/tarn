<script lang="ts">
  import { liveSearch, peekSearch, cacheKey } from '../search-cache';
  import type { SearchResults, ShelfItem, Track, ArtistCard, PlaylistCard } from '../types';
  import { itemToTrack } from '../cover';
  import { artistsLine, fmtTime } from '../format';
  import Cover from './Cover.svelte';
  import Hi from './Hi.svelte';
  import { Search01Icon, PlayIcon, ArrowRight01Icon } from '../icons';

  let {
    value = $bindable(''),
    autofocus = false,
    placeholder = 'Search',
    onSubmit,
    onPlay,
    onOpen,
  }: {
    value?: string;
    autofocus?: boolean;
    placeholder?: string;
    onSubmit: (q: string) => void;
    onPlay: (track: Track, queue: Track[]) => void;
    onOpen: (item: ShelfItem) => void;
  } = $props();

  let open = $state(false);
  let suggestions = $state<string[]>([]);
  let results = $state<SearchResults | null>(null);
  let inflight = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let gen = 0;
  let root: HTMLDivElement | undefined = $state();
  let bodyEl: HTMLDivElement | undefined = $state();
  let active = $state(-1);

  const SUG_MAX = 4;
  const SONG_MAX = 4;
  const ARTIST_MAX = 3;
  const ALBUM_MAX = 3;

  let sugList = $derived(suggestions.slice(0, SUG_MAX));
  let songList = $derived((results?.songs ?? []).slice(0, SONG_MAX));
  let artistList = $derived((results?.artists ?? []).slice(0, ARTIST_MAX));
  let albumList = $derived((results?.albums ?? []).slice(0, ALBUM_MAX));
  let top = $derived(results?.top ?? null);

  type Hit =
    | { kind: 'sug'; q: string }
    | { kind: 'top' }
    | { kind: 'song'; i: number }
    | { kind: 'artist'; i: number }
    | { kind: 'album'; i: number }
    | { kind: 'all' };

  let hits = $derived.by((): Hit[] => {
    const out: Hit[] = [];
    for (const q of sugList) out.push({ kind: 'sug', q });
    if (top) out.push({ kind: 'top' });
    for (let i = 0; i < songList.length; i++) out.push({ kind: 'song', i });
    for (let i = 0; i < artistList.length; i++) out.push({ kind: 'artist', i });
    for (let i = 0; i < albumList.length; i++) out.push({ kind: 'album', i });
    if (out.length) out.push({ kind: 'all' });
    return out;
  });

  let hasDrop = $derived(open && !!value.trim() && hits.length > 0);

  function apply(hit: { suggestions: string[]; results: SearchResults | null }) {
    suggestions = hit.suggestions ?? [];
    results = hit.results;
    active = -1;
  }

  function onInput(q: string) {
    value = q;
    clearTimeout(timer);
    const t = q.trim();
    if (!t) {
      suggestions = [];
      results = null;
      inflight = false;
      active = -1;
      return;
    }
    const cached = peekSearch(t);
    if (cached) apply(cached);
    const my = ++gen;
    inflight = !cached;
    timer = setTimeout(() => {
      liveSearch(t)
        .then((hit) => {
          if (my !== gen || cacheKey(value) !== cacheKey(t)) return;
          apply(hit);
        })
        .catch(() => {})
        .finally(() => {
          if (my === gen) inflight = false;
        });
    }, 180);
  }

  function submit(q?: string) {
    const qq = (q ?? value).trim();
    if (!qq) return;
    value = qq;
    open = false;
    onSubmit(qq);
  }

  function playSong(t: Track, ev?: Event) {
    ev?.stopPropagation();
    ev?.preventDefault();
    const queue = results?.songs?.length ? results.songs : [t];
    onPlay(t, queue);
    open = false;
  }

  function openItem(item: ShelfItem, ev?: Event) {
    ev?.stopPropagation();
    ev?.preventDefault();
    open = false;
    if (item.kind === 'track' || item.kind === 'video') {
      playSong(itemToTrack(item), ev);
      return;
    }
    onOpen(item);
  }


  function asArtist(a: ArtistCard): ShelfItem {
    return {
      kind: 'artist', id: a.id, title: a.name, subtitle: null, cover: a.cover,
      artists: [], artist_ids: [a.id], album: null, album_id: null, duration: null,
      playlist_id: null, color: null, browse_id: a.id, params: null, year: null,
      album_type: null, track_count: null, is_video: false,
    };
  }
  function asAlbum(a: PlaylistCard): ShelfItem {
    return {
      kind: 'album', id: a.id, title: a.name, subtitle: a.subtitle ?? null, cover: a.cover,
      artists: [], artist_ids: [], album: a.name, album_id: a.id, duration: null,
      playlist_id: null, color: null, browse_id: a.id, params: null, year: a.year ?? null,
      album_type: a.album_type ?? null, track_count: null, is_video: false,
    };
  }

  function kindLabel(k: string | undefined): string {
    switch (k) {
      case 'track': return 'Song';
      case 'video': return 'Video';
      case 'artist': return 'Artist';
      case 'album': return 'Album';
      case 'playlist': return 'Playlist';
      default: return k ? k[0].toUpperCase() + k.slice(1) : '';
    }
  }

  function activate(h: Hit) {
    switch (h.kind) {
      case 'sug': submit(h.q); break;
      case 'top': if (top) openItem(top); break;
      case 'song': playSong(songList[h.i]); break;
      case 'artist': open = false; onOpen(asArtist(artistList[h.i])); break;
      case 'album': open = false; onOpen(asAlbum(albumList[h.i])); break;
      case 'all': submit(); break;
    }
  }

  function move(delta: number) {
    if (!hits.length) return;
    const n = hits.length;
    active = active < 0 ? (delta > 0 ? 0 : n - 1) : (active + delta + n) % n;
    queueMicrotask(() => {
      const el = bodyEl?.querySelector<HTMLElement>(`[data-hit="${active}"]`);
      el?.scrollIntoView({ block: 'nearest' });
    });
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      if (hasDrop) {
        e.preventDefault();
        move(1);
      }
    } else if (e.key === 'ArrowUp') {
      if (hasDrop) {
        e.preventDefault();
        move(-1);
      }
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (hasDrop && active >= 0 && hits[active]) activate(hits[active]);
      else submit();
    } else if (e.key === 'Escape') {
      open = false;
      (e.currentTarget as HTMLInputElement).blur();
    }
  }

  function isOn(kind: Hit['kind'], i?: number): boolean {
    if (active < 0) return false;
    const h = hits[active];
    if (!h || h.kind !== kind) return false;
    if (kind === 'song' || kind === 'artist' || kind === 'album') {
      return (h as { i: number }).i === i;
    }
    return true;
  }

  function hitIndex(kind: Hit['kind'], i?: number): number {
    return hits.findIndex((h) => {
      if (h.kind !== kind) return false;
      if (kind === 'song' || kind === 'artist' || kind === 'album') {
        return (h as { i: number }).i === i;
      }
      return true;
    });
  }

  $effect(() => {
    if (!open) return;
    const onDoc = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) open = false;
    };
    document.addEventListener('pointerdown', onDoc, true);
    return () => document.removeEventListener('pointerdown', onDoc, true);
  });
</script>

<div class="search-bar" bind:this={root}>
  <div class="search-wrap">
    <span class="sic"><Hi icon={Search01Icon} size={20} /></span>
    <!-- svelte-ignore a11y_autofocus -->
    <input
      id="q"
      {placeholder}
      {autofocus}
      role="combobox"
      aria-autocomplete="list"
      aria-expanded={hasDrop}
      aria-controls="sug-list"
      aria-activedescendant={active >= 0 ? `sug-hit-${active}` : undefined}
      autocomplete="off"
      spellcheck="false"
      bind:value
      oninput={(e) => onInput(e.currentTarget.value)}
      onfocus={() => (open = true)}
      onkeydown={onKey}
    />
    {#if hasDrop}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="sug sug-rich"
        id="sug-list"
        role="listbox"
        tabindex="-1"
        onmousedown={(e) => e.preventDefault()}
      >
        <div class="sug-body" bind:this={bodyEl}>
          {#if sugList.length}
            <div class="sug-sec">
              {#each sugList as s, i}
                {@const idx = hitIndex('sug', i)}
                <button
                  class="sug-text"
                  class:on={isOn('sug') && hits[active] && (hits[active] as { q?: string }).q === s}
                  type="button"
                  role="option"
                  id="sug-hit-{idx}"
                  data-hit={idx}
                  aria-selected={isOn('sug') && (hits[active] as { q?: string }).q === s}
                  onclick={() => submit(s)}
                >
                  <span class="sug-ico"><Hi icon={Search01Icon} size={16} /></span>
                  <span class="sug-t">{s}</span>
                </button>
              {/each}
            </div>
          {/if}

          {#if top}
            {@const idx = hitIndex('top')}
            <div class="sug-label">Top result</div>
            <button
              class="sug-top"
              class:on={isOn('top')}
              class:artist={top.kind === 'artist'}
              type="button"
              role="option"
              id="sug-hit-{idx}"
              data-hit={idx}
              aria-selected={isOn('top')}
              onclick={(e) => openItem(top, e)}
            >
              <span class="sug-cov top">
                <Cover src={top.cover} size={80} round={top.kind === 'artist'} alt="" />
                <span class="play-ov"><Hi icon={PlayIcon} size={28} /></span>
              </span>
              <span class="sug-meta">
                <span class="sug-t">{top.title}</span>
                <span class="sug-s">{kindLabel(top.kind)}{top.subtitle || artistsLine(top.artists) ? ` · ${top.subtitle || artistsLine(top.artists)}` : ''}</span>
              </span>
              <span class="sug-play" aria-hidden="true">
                <Hi icon={PlayIcon} size={18} />
              </span>
            </button>
          {/if}

          {#if songList.length}
            <div class="sug-label">Songs</div>
            {#each songList as t, i}
              {@const idx = hitIndex('song', i)}
              <button
                class="sug-row"
                class:on={isOn('song', i)}
                type="button"
                role="option"
                id="sug-hit-{idx}"
                data-hit={idx}
                aria-selected={isOn('song', i)}
                onclick={(e) => playSong(t, e)}
              >
                <span class="sug-cov">
                  <Cover src={t.cover} size={48} alt="" />
                  <span class="play-ov"><Hi icon={PlayIcon} size={18} /></span>
                </span>
                <span class="sug-meta">
                  <span class="sug-t">{t.title}</span>
                  <span class="sug-s">{artistsLine(t.artists)}</span>
                </span>
                {#if t.duration}
                  <span class="sug-dur">{fmtTime(t.duration)}</span>
                {/if}
              </button>
            {/each}
          {/if}

          {#if artistList.length}
            <div class="sug-label">Artists</div>
            {#each artistList as a, i}
              {@const idx = hitIndex('artist', i)}
              <button
                class="sug-row"
                class:on={isOn('artist', i)}
                type="button"
                role="option"
                id="sug-hit-{idx}"
                data-hit={idx}
                aria-selected={isOn('artist', i)}
                onclick={() => { open = false; onOpen(asArtist(a)); }}
              >
                <span class="sug-cov round">
                  <Cover src={a.cover} size={48} round alt="" />
                </span>
                <span class="sug-meta">
                  <span class="sug-t">{a.name}</span>
                  <span class="sug-s">Artist</span>
                </span>
              </button>
            {/each}
          {/if}

          {#if albumList.length}
            <div class="sug-label">Albums</div>
            {#each albumList as a, i}
              {@const idx = hitIndex('album', i)}
              <button
                class="sug-row"
                class:on={isOn('album', i)}
                type="button"
                role="option"
                id="sug-hit-{idx}"
                data-hit={idx}
                aria-selected={isOn('album', i)}
                onclick={() => { open = false; onOpen(asAlbum(a)); }}
              >
                <span class="sug-cov">
                  <Cover src={a.cover} size={48} alt="" />
                </span>
                <span class="sug-meta">
                  <span class="sug-t">{a.name}</span>
                  <span class="sug-s">{a.subtitle || a.album_type || 'Album'}{a.year ? ` · ${a.year}` : ''}</span>
                </span>
              </button>
            {/each}
          {/if}
        </div>

        <button
          class="sug-all"
          class:on={isOn('all')}
          type="button"
          role="option"
          id="sug-hit-{hitIndex('all')}"
          data-hit={hitIndex('all')}
          aria-selected={isOn('all')}
          onclick={() => submit()}
        >
          See all results <Hi icon={ArrowRight01Icon} size={16} />
        </button>
      </div>
    {/if}
  </div>
</div>
