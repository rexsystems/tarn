<script lang="ts">
  import { liveSearch, peekSearch, cacheKey } from '../search-cache';
  import type { SearchResults, ShelfItem, Track } from '../types';
  import { itemToTrack } from '../cover';
  import { artistsLine } from '../format';
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

  let hasDrop = $derived(
    open &&
      !!value.trim() &&
      (suggestions.length > 0 ||
        !!results?.top ||
        (results?.songs?.length ?? 0) > 0 ||
        (results?.artists?.length ?? 0) > 0 ||
        (results?.albums?.length ?? 0) > 0),
  );

  function apply(hit: { suggestions: string[]; results: SearchResults | null }) {
    suggestions = hit.suggestions ?? [];
    results = hit.results;
  }

  function onInput(q: string) {
    value = q;
    clearTimeout(timer);
    const t = q.trim();
    if (!t) {
      suggestions = [];
      results = null;
      inflight = false;
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

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      submit();
    } else if (e.key === 'Escape') {
      open = false;
      (e.currentTarget as HTMLInputElement).blur();
    }
  }

  function asArtist(id: string, title: string, cover: string | null): ShelfItem {
    return {
      kind: 'artist', id, title, subtitle: null, cover,
      artists: [], artist_ids: [id], album: null, album_id: null, duration: null,
      playlist_id: null, color: null, browse_id: id, params: null, year: null,
      album_type: null, track_count: null, is_video: false,
    };
  }
  function asAlbum(id: string, title: string, cover: string | null, subtitle: string | null, year: number | null, album_type: string | null): ShelfItem {
    return {
      kind: 'album', id, title, subtitle, cover,
      artists: [], artist_ids: [], album: title, album_id: id, duration: null,
      playlist_id: null, color: null, browse_id: id, params: null, year,
      album_type, track_count: null, is_video: false,
    };
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
        onmousedown={(e) => e.preventDefault()}
      >
        {#if suggestions.length}
          <div class="sug-sec">
            {#each suggestions.slice(0, 6) as s}
              <button class="sug-text" type="button" onclick={() => submit(s)}>{s}</button>
            {/each}
          </div>
        {/if}

        {#if results?.top}
          {@const top = results.top}
          <div class="sug-label">Top result</div>
          <button
            class="sug-top"
            type="button"
            onclick={(e) => openItem(top, e)}
          >
            <span class="sug-cov">
              <Cover src={top.cover} size={56} round={top.kind === 'artist'} alt="" />
              {#if top.kind === 'track' || top.kind === 'video'}
                <span class="play-ov"><Hi icon={PlayIcon} size={20} /></span>
              {/if}
            </span>
            <span class="sug-meta">
              <span class="sug-k">{top.kind}</span>
              <span class="sug-t">{top.title}</span>
              <span class="sug-s">{top.subtitle || artistsLine(top.artists)}</span>
            </span>
          </button>
        {/if}

        {#if results?.songs?.length}
          <div class="sug-label">Songs</div>
          {#each results.songs.slice(0, 5) as t}
            <button class="sug-row" type="button" onclick={(e) => playSong(t, e)}>
              <span class="sug-cov">
                <Cover src={t.cover} size={40} alt="" />
                <span class="play-ov"><Hi icon={PlayIcon} size={16} /></span>
              </span>
              <span class="sug-meta">
                <span class="sug-t">{t.title}</span>
                <span class="sug-s">{artistsLine(t.artists)}</span>
              </span>
            </button>
          {/each}
        {/if}

        {#if results?.artists?.length}
          <div class="sug-label">Artists</div>
          {#each results.artists.slice(0, 3) as a}
            <button
              class="sug-row"
              type="button"
              onclick={() => { open = false; onOpen(asArtist(a.id, a.name, a.cover)); }}
            >
              <span class="sug-cov">
                <Cover src={a.cover} size={40} round alt="" />
              </span>
              <span class="sug-meta">
                <span class="sug-t">{a.name}</span>
                <span class="sug-s">Artist</span>
              </span>
            </button>
          {/each}
        {/if}

        {#if results?.albums?.length}
          <div class="sug-label">Albums</div>
          {#each results.albums.slice(0, 3) as a}
            <button
              class="sug-row"
              type="button"
              onclick={() => { open = false; onOpen(asAlbum(a.id, a.name, a.cover, a.subtitle ?? null, a.year ?? null, a.album_type ?? null)); }}
            >
              <span class="sug-cov">
                <Cover src={a.cover} size={40} alt="" />
              </span>
              <span class="sug-meta">
                <span class="sug-t">{a.name}</span>
                <span class="sug-s">{a.subtitle || a.album_type || 'Album'}{a.year ? ` · ${a.year}` : ''}</span>
              </span>
            </button>
          {/each}
        {/if}

        <button class="sug-all" type="button" onclick={() => submit()}>
          See all results <Hi icon={ArrowRight01Icon} size={16} />
        </button>
      </div>
    {/if}
  </div>
</div>
