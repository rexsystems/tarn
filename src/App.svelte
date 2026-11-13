<script lang="ts">
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { api } from './lib/api';
  import type {
    ArtistPage, AuthStatus, Collection, Explore, GenrePage, Home, Library,
    Lyrics, PlayerState, PlaylistCard, Related, SearchResults, Settings as SettingsT,
    Shelf, ShelfItem, Track, View, CtxTarget,
  } from './lib/types';
  import { itemToTrack } from './lib/cover';
  import { artistsLine, fmtCount, fmtTime, shareUrl } from './lib/format';
  import Cover from './lib/components/Cover.svelte';
  import Hi from './lib/components/Hi.svelte';
  import IconBtn from './lib/components/IconBtn.svelte';
  import ShelfRow from './lib/components/ShelfRow.svelte';
  import TrackTable from './lib/components/TrackTable.svelte';
  import ContextMenu from './lib/components/ContextMenu.svelte';
  import Slider from './lib/components/Slider.svelte';
  import {
    Home01Icon, Search01Icon, Compass01Icon, LibraryIcon, Settings02Icon,
    PlayIcon, PauseIcon, PreviousIcon, NextIcon, ShuffleIcon, RepeatIcon, RepeatOne01Icon,
    VolumeHighIcon, VolumeMute01Icon, FavouriteIcon, ThumbsDownIcon, Queue02Icon,
    Radio02Icon, Cancel01Icon, HistoryIcon, UserCircle02Icon, Logout03Icon,
    Login03Icon, MicVocalIcon, Bookmark02Icon, Notification01Icon, Globe02Icon,
    LanguageCircleIcon, Album02Icon, Playlist01Icon, MusicNote01Icon,
  } from './lib/icons';

  let view = $state<View>({ id: 'home' });
  let home = $state<Home | null>(null);
  let explore = $state<Explore | null>(null);
  let searchQ = $state('');
  let searchFocus = $state(false);
  let suggestions = $state<string[]>([]);
  let results = $state<SearchResults | null>(null);
  let searchTab = $state<'top' | 'songs' | 'videos' | 'albums' | 'artists' | 'playlists' | 'community'>('top');
  let artist = $state<ArtistPage | null>(null);
  let collection = $state<Collection | null>(null);
  let genre = $state<GenrePage | null>(null);
  let charts = $state<Home | null>(null);
  let releases = $state<Home | null>(null);
  let library = $state<Library | null>(null);
  let history = $state<Track[] | null>(null);
  let auth = $state<AuthStatus>({ signed_in: false, mock: false, account: null, store: null });
  let settings = $state<SettingsT>({ quality: 'high', language: 'en', region: 'US', notifications: true, autoplay: true });
  let player = $state<PlayerState>({
    track: null, playing: false, loading: false, position: 0, duration: 0,
    volume: 0.85, queue: [], index: 0, error: null, accent: null,
    shuffle: false, repeat: 'off', liked: false, disliked: false, autoplay: true,
  });
  let loading = $state(false);
  let err = $state<string | null>(null);
  let chip = $state<string | null>(null);
  let panel = $state<null | 'queue' | 'lyrics' | 'related'>(null);
  let expanded = $state(false);
  let scrub = $state<number | null>(null);
  let lyrics = $state<Lyrics | null>(null);
  let related = $state<Related | null>(null);
  let ctx = $state<CtxTarget | null>(null);
  let addTitle = $state('');

  $effect(() => {
    if (player.accent) {
      const [r, g, b] = player.accent;
      document.documentElement.style.setProperty('--accent-rgb', `${r}, ${g}, ${b}`);
      document.documentElement.style.setProperty('--accent', `rgb(${r}, ${g}, ${b})`);
      document.documentElement.style.setProperty('--accent-dim', `rgba(${r}, ${g}, ${b}, 0.18)`);
    }
  });

  onMount(() => {
    const unsubs: Array<() => void> = [];
    (async () => {
      try {
        player = await api.state();
        auth = await api.auth();
        settings = await api.settings();
      } catch {}
      await loadHome();
      if (auth.signed_in) {
        api.library().then((l) => (library = l)).catch(() => {});
      }
    })();
    listen<PlayerState>('player', (e) => {
      player = e.payload;
    }).then((u) => unsubs.push(u));
    listen('auth', () => {
      api.auth().then((a) => {
        auth = a;
        if (a.signed_in) {
          api.library().then((l) => (library = l)).catch(() => {});
          loadHome();
        }
      });
    }).then((u) => unsubs.push(u));

    const keys = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      const typing = tag === 'INPUT' || tag === 'TEXTAREA' || (e.target as HTMLElement)?.isContentEditable;
      if (e.key === '/' && !typing) {
        e.preventDefault();
        go({ id: 'search' });
        queueMicrotask(() => document.getElementById('q')?.focus());
        return;
      }
      if (typing) return;
      if (e.key === ' ' || e.code === 'MediaPlayPause') {
        e.preventDefault();
        api.toggle();
      } else if (e.key === 'ArrowRight' && e.ctrlKey) {
        api.next();
      } else if (e.key === 'ArrowLeft' && e.ctrlKey) {
        api.prev();
      } else if (e.key === 'ArrowRight') {
        api.seek(player.position + 5);
      } else if (e.key === 'ArrowLeft') {
        api.seek(Math.max(0, player.position - 5));
      } else if (e.code === 'MediaTrackNext') {
        api.next();
      } else if (e.code === 'MediaTrackPrevious') {
        api.prev();
      } else if (e.key === 'Escape') {
        if (expanded) expanded = false;
        else if (panel) panel = null;
        else if (ctx) ctx = null;
      }
    };
    window.addEventListener('keydown', keys);
    return () => {
      unsubs.forEach((u) => u());
      window.removeEventListener('keydown', keys);
    };
  });

  function go(v: View) {
    view = v;
    err = null;
    ctx = null;
    if (v.id === 'home' && !home) loadHome();
    if (v.id === 'explore' && !explore) loadExplore();
    if (v.id === 'library') loadLibrary();
    if (v.id === 'artist') loadArtist(v.artistId);
    if (v.id === 'album') loadAlbum(v.albumId);
    if (v.id === 'playlist') loadPlaylist(v.playlistId);
    if (v.id === 'genre') loadGenre(v.genreId);
    if (v.id === 'charts') loadCharts();
    if (v.id === 'new-releases') loadReleases();
    if (v.id === 'history') loadHistory();
    if (v.id === 'search') queueMicrotask(() => document.getElementById('q')?.focus());
  }

  async function loadHome(params?: string | null) {
    loading = true;
    try {
      home = await api.home(params);
    } catch (e) {
      err = String(e);
    } finally {
      loading = false;
    }
  }
  async function loadExplore() {
    loading = true;
    try { explore = await api.explore(); } catch (e) { err = String(e); } finally { loading = false; }
  }
  async function loadArtist(id: string) {
    loading = true; artist = null;
    try { artist = await api.artist(id); } catch (e) { err = String(e); } finally { loading = false; }
  }
  async function loadAlbum(id: string) {
    loading = true; collection = null;
    try { collection = await api.album(id); } catch (e) { err = String(e); } finally { loading = false; }
  }
  async function loadPlaylist(id: string) {
    loading = true; collection = null;
    try { collection = await api.playlist(id); } catch (e) { err = String(e); } finally { loading = false; }
  }
  async function loadGenre(id: string) {
    loading = true; genre = null;
    try { genre = await api.genre(id); } catch (e) { err = String(e); } finally { loading = false; }
  }
  async function loadCharts() {
    loading = true;
    try { charts = await api.charts(); } catch (e) { err = String(e); } finally { loading = false; }
  }
  async function loadReleases() {
    loading = true;
    try { releases = await api.newReleases(); } catch (e) { err = String(e); } finally { loading = false; }
  }
  async function loadLibrary() {
    try { library = await api.library(); } catch { library = { signed_in: auth.signed_in, mock: auth.mock, account: auth.account, playlists: [], liked: [], albums: [], artists: [], recents: [] }; }
  }
  async function loadHistory() {
    loading = true;
    try { history = await api.history(); } catch (e) { err = String(e); history = []; } finally { loading = false; }
  }

  let sugTimer: ReturnType<typeof setTimeout> | undefined;
  function onQuery(q: string) {
    searchQ = q;
    clearTimeout(sugTimer);
    if (!q.trim()) { suggestions = []; return; }
    sugTimer = setTimeout(() => {
      api.suggestions(q).then((s) => (suggestions = s)).catch(() => {});
    }, 160);
  }
  async function runSearch(q?: string) {
    const qq = (q ?? searchQ).trim();
    if (!qq) return;
    searchQ = qq;
    suggestions = [];
    searchFocus = false;
    loading = true;
    view = { id: 'search' };
    try { results = await api.searchAll(qq); searchTab = 'top'; } catch (e) { err = String(e); } finally { loading = false; }
  }

  async function playTracks(tracks: Track[], index: number) {
    if (!tracks.length) return;
    const t = tracks[index] ?? tracks[0];
    try { await api.play(t, tracks, index); } catch (e) { err = String(e); }
  }
  function playOne(t: Track) { playTracks([t, ...player.queue.filter((x) => x.id !== t.id)], 0); }

  function shuffled<T>(xs: T[]): T[] {
    const a = xs.slice();
    for (let i = a.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [a[i], a[j]] = [a[j], a[i]];
    }
    return a;
  }

  function playTopShuffled(top: ShelfItem) {
    const songs = results?.songs?.length ? results.songs : [itemToTrack(top)];
    api.setShuffle(true);
    playTracks(shuffled(songs), 0);
  }

  async function playCollection(item: ShelfItem, shuffle: boolean) {
    try {
      let tracks: Track[] = [];
      if (item.kind === 'artist') {
        const a = await api.artist(item.id || item.browse_id || '');
        tracks = a.tracks;
      } else if (item.kind === 'album') {
        const c = await api.album(item.id || item.browse_id || '');
        tracks = c.tracks;
      } else if (item.kind === 'playlist') {
        const c = await api.playlist(item.id);
        tracks = c.tracks;
      } else if (item.kind === 'track' || item.kind === 'video') {
        tracks = [itemToTrack(item)];
      }
      if (!tracks.length) { openItem(item); return; }
      if (shuffle) api.setShuffle(true);
      playTracks(shuffle ? shuffled(tracks) : tracks, 0);
    } catch (e) { err = String(e); }
  }

  function openItem(item: ShelfItem) {
    if (item.kind === 'artist') go({ id: 'artist', artistId: item.id || item.browse_id || '' });
    else if (item.kind === 'album') go({ id: 'album', albumId: item.id || item.browse_id || '' });
    else if (item.kind === 'playlist') go({ id: 'playlist', playlistId: item.id });
    else if (item.kind === 'mood') {
      const id = item.params || item.id;
      const bid = item.browse_id || '';
      if (bid === 'FEmusic_moods_and_genres_category' || (id && id.length > 8 && !id.startsWith('FE'))) {
        go({ id: 'genre', genreId: id, title: item.title });
      } else if (bid === 'FEmusic_home' || item.params) {
        go({ id: 'home' });
        chip = item.title;
        loadHome(item.params);
      } else {
        go({ id: 'genre', genreId: id, title: item.title });
      }
    } else if (item.kind === 'track' || item.kind === 'video') {
      playOne(itemToTrack(item));
    }
  }

  function openChip(c: { title: string; params?: string | null; browse_id?: string | null }) {
    chip = c.title;
    if (c.browse_id === 'FEmusic_moods_and_genres_category' && c.params) {
      go({ id: 'genre', genreId: c.params, title: c.title });
      return;
    }
    loadHome(c.params);
    go({ id: 'home' });
  }

  function contextItem(e: MouseEvent, item: ShelfItem) {
    e.preventDefault();
    const track = item.kind === 'track' || item.kind === 'video' ? itemToTrack(item) : undefined;
    ctx = {
      x: e.clientX, y: e.clientY, track, item,
      albumId: item.kind === 'album' ? item.id : item.album_id || undefined,
      artistId: item.kind === 'artist' ? item.id : item.artist_ids?.[0],
      playlistId: item.kind === 'playlist' ? item.id : undefined,
    };
  }
  function contextTrack(e: MouseEvent, t: Track) {
    e.preventDefault();
    ctx = { x: e.clientX, y: e.clientY, track: t, albumId: t.album_id || undefined, artistId: t.artist_ids?.[0] };
  }

  async function like(action: string) {
    const id = player.track?.id;
    if (!id) return;
    try {
      const st = await api.like(id, action);
      player.liked = st.liked;
      player.disliked = st.disliked;
    } catch (e) { err = String(e); }
  }
  async function toggleLike() {
    await like(player.liked ? 'removelike' : 'like');
  }
  async function toggleDislike() {
    await like(player.disliked ? 'removelike' : 'dislike');
  }

  async function openPanel(tab: 'queue' | 'lyrics' | 'related') {
    panel = panel === tab ? null : tab;
    const id = player.track?.id;
    if (tab === 'lyrics' && id) {
      try { lyrics = await api.lyrics(id); } catch { lyrics = { source: null, lines: [], synced: false }; }
    }
    if (tab === 'related' && id) {
      try { related = await api.related(id); } catch { related = null; }
    }
  }

  $effect(() => {
    const id = player.track?.id;
    if (panel === 'lyrics' && id) {
      api.lyrics(id).then((l) => (lyrics = l)).catch(() => {});
    }
    if (panel === 'related' && id) {
      api.related(id).then((r) => (related = r)).catch(() => {});
    }
  });

  function commitSeek(seconds: number) {
    scrub = null;
    api.seek(seconds);
  }

  function previewSeek(seconds: number | null) {
    scrub = seconds;
  }

  async function share(t?: Track) {
    const url = shareUrl({
      video: t?.id || ctx?.track?.id,
      album: ctx?.albumId,
      playlist: ctx?.playlistId,
      artist: ctx?.artistId,
    });
    try { await navigator.clipboard.writeText(url); } catch {}
  }

  async function addToPlaylist(pid: string, videoId?: string) {
    const vid = videoId || ctx?.track?.id;
    if (!vid) return;
    if (!pid) {
      const title = prompt('Playlist name');
      if (!title) return;
      try {
        const id = await api.playlistCreate(title);
        await api.playlistAdd(id, vid);
        loadLibrary();
      } catch (e) { err = String(e); }
      return;
    }
    try { await api.playlistAdd(pid, vid); } catch (e) { err = String(e); }
  }

  function goArtist(id: string, name?: string) {
    if (id) go({ id: 'artist', artistId: id });
    else if (name) { searchQ = name; runSearch(name); }
  }

  let activeLyric = $derived.by(() => {
    if (!lyrics?.synced) return -1;
    const ms = player.position * 1000;
    let idx = -1;
    for (let i = 0; i < lyrics.lines.length; i++) {
      const s = lyrics.lines[i].start_ms;
      if (s != null && s <= ms) idx = i;
    }
    return idx;
  });

  $effect(() => {
    if (lyrics?.synced && activeLyric >= 0) {
      const el = document.querySelector(`.ly[data-i="${activeLyric}"]`);
      el?.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }
  });

  let navOn = (id: string) => view.id === id;
</script>

<div class="app" style="--player: 112px; --side: 248px;">
  <aside class="sidebar">
    <div class="brand">
      <img src="/logo.png" width="44" height="44" alt="" />
      <span class="word">Tarn</span>
    </div>
    <nav class="nav">
      <button class="nav-item {navOn('home') ? 'on' : ''}" onclick={() => go({ id: 'home' })} type="button">
        <span class="ic"><Hi icon={Home01Icon} size={24} /></span> Home
      </button>
      <button class="nav-item {navOn('explore') || view.id === 'charts' || view.id === 'new-releases' || view.id === 'genre' ? 'on' : ''}" onclick={() => go({ id: 'explore' })} type="button">
        <span class="ic"><Hi icon={Compass01Icon} size={24} /></span> Explore
      </button>
      <button class="nav-item {navOn('search') ? 'on' : ''}" onclick={() => go({ id: 'search' })} type="button">
        <span class="ic"><Hi icon={Search01Icon} size={24} /></span> Search
      </button>
      <button class="nav-item {navOn('library') || view.id === 'history' ? 'on' : ''}" onclick={() => go({ id: 'library' })} type="button">
        <span class="ic"><Hi icon={LibraryIcon} size={24} /></span> Library
      </button>
    </nav>
    <div class="side-scroll">
      {#if auth.signed_in && library?.playlists?.length}
        <div class="side-label">Playlists</div>
        {#each library.playlists as p}
          <button class="side-pl" type="button" onclick={() => go({ id: 'playlist', playlistId: p.id })}>
            <Cover src={p.cover} size={36} alt="" />
            <span class="clamp one">{p.name}</span>
          </button>
        {/each}
      {/if}
    </div>
    <div class="side-foot">
      <button class="nav-item {navOn('settings') ? 'on' : ''}" onclick={() => go({ id: 'settings' })} type="button">
        <span class="ic"><Hi icon={Settings02Icon} size={22} /></span> Settings
      </button>
      {#if auth.signed_in && auth.account}
        <div class="acct">
          {#if auth.account.photo}
            <Cover src={auth.account.photo} size={36} round class="av" alt="" />
          {:else}
            <span class="av"><Hi icon={UserCircle02Icon} size={22} /></span>
          {/if}
          <div style="min-width:0">
            <div class="nm clamp one">
              {auth.account.name}
              {#if auth.mock}<span class="badge">Preview</span>{/if}
            </div>
            {#if auth.account.handle}<div class="hd">{auth.account.handle}</div>{/if}
          </div>
        </div>
      {:else}
        <button class="acct" type="button" onclick={() => api.openLogin()}>
          <span class="av"><Hi icon={Login03Icon} size={22} /></span>
          <div class="nm">Sign in</div>
        </button>
      {/if}
    </div>
  </aside>

  <main class="main {panel ? 'with-panel' : ''}" class:is-hidden={expanded}>
    <div class="page">
      {#if err}<div class="err">{err}</div>{/if}

      {#if view.id === 'home'}
        {#if home?.chips?.length}
          <div class="chips">
            {#each home.chips as c}
              <button class="chip {chip === c.title || c.selected ? 'on' : ''}" type="button" onclick={() => openChip(c)}>{c.title}</button>
            {/each}
          </div>
        {/if}
        {#if loading && !home}<div class="spin" style="margin:40px auto"></div>{/if}
        {#each home?.shelves ?? [] as s}
          <ShelfRow shelf={s} onPlayItems={playTracks} onOpen={openItem} onContext={contextItem} />
        {/each}

      {:else if view.id === 'explore'}
        <h1 class="page-title">Explore</h1>
        <div class="explore-links">
          <button type="button" onclick={() => go({ id: 'new-releases' })}><Hi icon={Album02Icon} size={24} /> New releases</button>
          <button type="button" onclick={() => go({ id: 'charts' })}><Hi icon={MusicNote01Icon} size={24} /> Charts</button>
          <button type="button" onclick={() => document.getElementById('moods')?.scrollIntoView({ behavior: 'smooth' })}><Hi icon={Compass01Icon} size={24} /> Moods & genres</button>
        </div>
        {#if loading && !explore}<div class="spin" style="margin:40px auto"></div>{/if}
        {#each (explore?.shelves ?? []).filter((s) => s.kind !== 'moods' && !s.title.toLowerCase().includes('mood')) as s}
          <ShelfRow shelf={s} onPlayItems={playTracks} onOpen={openItem} onContext={contextItem} />
        {/each}
        {#if explore?.moods?.length}
          <section class="shelf" id="moods">
            <div class="shelf-head"><h2 class="shelf-title">Moods & genres</h2></div>
            <div class="mood-grid">
              {#each explore.moods as m}
                <button class="mood" style="background:{m.color || '#3a4a5c'}" type="button" onclick={() => openItem(m)}>{m.title}</button>
              {/each}
            </div>
          </section>
        {/if}

      {:else if view.id === 'charts' || view.id === 'new-releases'}
        <h1 class="page-title">{view.id === 'charts' ? 'Charts' : 'New releases'}</h1>
        {#each (view.id === 'charts' ? charts : releases)?.shelves ?? [] as s}
          <ShelfRow shelf={s} onPlayItems={playTracks} onOpen={openItem} onContext={contextItem} />
        {/each}

      {:else if view.id === 'search'}
        <div class="search-bar">
          <div class="search-wrap">
            <span class="sic"><Hi icon={Search01Icon} size={20} /></span>
            <input
              id="q"
              placeholder="Search"
              bind:value={searchQ}
              oninput={(e) => onQuery(e.currentTarget.value)}
              onfocus={() => (searchFocus = true)}
              onblur={() => setTimeout(() => (searchFocus = false), 180)}
              onkeydown={(e) => e.key === 'Enter' && runSearch()}
            />
            {#if searchFocus && suggestions.length}
              <div class="sug">
                {#each suggestions as s}
                  <button type="button" onclick={() => runSearch(s)}>{s}</button>
                {/each}
              </div>
            {/if}
          </div>
        </div>
        {#if results}
          <div class="tabs">
            {#each [
              ['top', 'Top'],
              ['songs', 'Songs'],
              ['videos', 'Videos'],
              ['albums', 'Albums'],
              ['artists', 'Artists'],
              ['playlists', 'Playlists'],
              ['community', 'Community'],
            ] as [id, label]}
              <button class="tab {searchTab === id ? 'on' : ''}" type="button" onclick={() => (searchTab = id as typeof searchTab)}>{label}</button>
            {/each}
          </div>
          {#if (searchTab === 'top') && results.top}
            {@const top = results.top}
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            <div
              class="top-card {top.kind === 'artist' ? 'artist' : ''}"
              role="button"
              tabindex="0"
              onclick={() => openItem(top)}
              onkeydown={(e) => e.key === 'Enter' && openItem(top)}
              oncontextmenu={(e) => contextItem(e, top)}
            >
              <Cover src={top.cover} size={160} round={top.kind === 'artist'} alt="" />
              <div>
                <div class="k">{top.kind}</div>
                <div class="ttl">{top.title}</div>
                <div class="sub">{top.subtitle || artistsLine(top.artists)}</div>
                <div class="acts">
                  {#if top.kind === 'track' || top.kind === 'video'}
                    <button class="btn" type="button" onclick={(e) => { e.stopPropagation(); playOne(itemToTrack(top)); }}><Hi icon={PlayIcon} size={18} /> Play</button>
                    <button class="btn ghost" type="button" onclick={(e) => { e.stopPropagation(); playTopShuffled(top); }}><Hi icon={ShuffleIcon} size={18} /> Shuffle</button>
                    <button class="btn ghost" type="button" onclick={(e) => { e.stopPropagation(); api.radio(top.id); }}><Hi icon={Radio02Icon} size={18} /> Radio</button>
                  {:else}
                    <button class="btn" type="button" onclick={(e) => { e.stopPropagation(); playCollection(top, false); }}><Hi icon={PlayIcon} size={18} /> Play</button>
                    <button class="btn ghost" type="button" onclick={(e) => { e.stopPropagation(); playCollection(top, true); }}><Hi icon={ShuffleIcon} size={18} /> Shuffle</button>
                    <button class="btn ghost" type="button" onclick={(e) => { e.stopPropagation(); openItem(top); }}>Open</button>
                  {/if}
                </div>
              </div>
            </div>
          {/if}
          {#if searchTab === 'top' || searchTab === 'songs'}
            {#if searchTab === 'top'}<h2 class="shelf-title" style="margin-bottom:12px">Songs</h2>{/if}
            <TrackTable
              tracks={results.songs}
              currentId={player.track?.id}
              playing={player.playing}
              onPlay={(t, i) => playTracks(results!.songs, i)}
              onArtist={(id, n) => goArtist(id, n)}
              onAlbum={(id) => go({ id: 'album', albumId: id })}
              onContext={(e, t) => contextTrack(e, t)}
            />
          {/if}
          {#if searchTab === 'videos'}
            <TrackTable tracks={results.videos} currentId={player.track?.id} playing={player.playing} showAlbum={false}
              onPlay={(t, i) => playTracks(results!.videos, i)}
              onArtist={(id, n) => goArtist(id, n)}
              onAlbum={(id) => go({ id: 'album', albumId: id })}
              onContext={(e, t) => contextTrack(e, t)} />
          {/if}
          {#if searchTab === 'top' || searchTab === 'albums'}
            {#if results.albums.length}
              {#if searchTab === 'top'}<h2 class="shelf-title" style="margin:28px 0 12px">Albums</h2>{/if}
              <div class="row">
                {#each results.albums as a}
                  <button class="card" type="button" onclick={() => go({ id: 'album', albumId: a.id })}>
                    <div class="art"><Cover src={a.cover} size={152} alt="" /></div>
                    <div class="c-a"><div class="title">{a.name}</div><div class="sub">{a.subtitle || a.album_type || ''}{a.year ? ` · ${a.year}` : ''}</div></div>
                  </button>
                {/each}
              </div>
            {/if}
          {/if}
          {#if searchTab === 'top' || searchTab === 'artists'}
            {#if results.artists.length}
              {#if searchTab === 'top'}<h2 class="shelf-title" style="margin:28px 0 12px">Artists</h2>{/if}
              <div class="row">
                {#each results.artists as a}
                  <button class="card artist" type="button" onclick={() => go({ id: 'artist', artistId: a.id })}>
                    <div class="art"><Cover src={a.cover} size={132} round alt="" /></div>
                    <div class="c-a"><div class="title">{a.name}</div></div>
                  </button>
                {/each}
              </div>
            {/if}
          {/if}
          {#if searchTab === 'top' || searchTab === 'playlists'}
            {#if results.playlists.length}
              {#if searchTab === 'top'}<h2 class="shelf-title" style="margin:28px 0 12px">Playlists</h2>{/if}
              <div class="row">
                {#each results.playlists as p}
                  <button class="card" type="button" onclick={() => go({ id: 'playlist', playlistId: p.id })}>
                    <div class="art"><Cover src={p.cover} size={152} alt="" /></div>
                    <div class="c-a"><div class="title">{p.name}</div><div class="sub">{p.subtitle || ''}</div></div>
                  </button>
                {/each}
              </div>
            {/if}
          {/if}
          {#if searchTab === 'community'}
            <div class="row">
              {#each results.community as p}
                <button class="card" type="button" onclick={() => go({ id: 'playlist', playlistId: p.id })}>
                  <div class="art"><Cover src={p.cover} size={152} alt="" /></div>
                  <div class="c-a"><div class="title">{p.name}</div><div class="sub">{p.subtitle || ''}</div></div>
                </button>
              {/each}
            </div>
          {/if}
        {/if}

      {:else if view.id === 'library'}
        <h1 class="page-title">Library {#if auth.mock}<span class="badge">Preview</span>{/if}</h1>
        {#if !auth.signed_in}
          <div class="empty"></div>
        {:else}
          <div class="tabs" style="padding-top:0">
            <button class="tab on" type="button" onclick={() => go({ id: 'history' })}><Hi icon={HistoryIcon} size={18} /> History</button>
          </div>
          {#if library?.liked?.length}
            <h2 class="shelf-title" style="margin-bottom:12px">Liked</h2>
            <TrackTable tracks={library.liked} currentId={player.track?.id} playing={player.playing}
              onPlay={(t, i) => playTracks(library!.liked, i)}
              onArtist={(id, n) => goArtist(id, n)}
              onAlbum={(id) => go({ id: 'album', albumId: id })}
              onContext={(e, t) => contextTrack(e, t)} />
          {/if}
          {#if library?.playlists?.length}
            <h2 class="shelf-title" style="margin:28px 0 12px">Playlists</h2>
            <div class="row">
              {#each library.playlists as p}
                <button class="card" type="button" onclick={() => go({ id: 'playlist', playlistId: p.id })}>
                  <div class="art"><Cover src={p.cover} size={152} alt="" /></div>
                  <div class="c-a"><div class="title">{p.name}</div><div class="sub">{p.track_count ? `${p.track_count}` : ''}</div></div>
                </button>
              {/each}
            </div>
          {/if}
          {#if library?.albums?.length}
            <h2 class="shelf-title" style="margin:28px 0 12px">Albums</h2>
            <div class="row">
              {#each library.albums as a}
                <button class="card" type="button" onclick={() => go({ id: 'album', albumId: a.id })}>
                  <div class="art"><Cover src={a.cover} size={152} alt="" /></div>
                  <div class="c-a"><div class="title">{a.name}</div><div class="sub">{a.subtitle || ''}</div></div>
                </button>
              {/each}
            </div>
          {/if}
          {#if library?.artists?.length}
            <h2 class="shelf-title" style="margin:28px 0 12px">Artists</h2>
            <div class="row">
              {#each library.artists as a}
                <button class="card artist" type="button" onclick={() => go({ id: 'artist', artistId: a.id })}>
                  <div class="art"><Cover src={a.cover} size={132} round alt="" /></div>
                  <div class="c-a"><div class="title">{a.name}</div></div>
                </button>
              {/each}
            </div>
          {/if}
        {/if}

      {:else if view.id === 'history'}
        <h1 class="page-title">History</h1>
        {#if history?.length}
          <TrackTable tracks={history} currentId={player.track?.id} playing={player.playing}
            onPlay={(t, i) => playTracks(history!, i)}
            onArtist={(id, n) => goArtist(id, n)}
            onAlbum={(id) => go({ id: 'album', albumId: id })}
            onContext={(e, t) => contextTrack(e, t)} />
        {/if}

      {:else if view.id === 'artist' && artist}
        <div class="hero" style="min-height:320px">
          {#if artist.header}
            <Cover src={artist.header} banner width={1240} height={320} class="hero-bg" alt="" />
          {:else if artist.cover}
            <Cover src={artist.cover} width={1240} height={320} class="hero-bg" alt="" />
          {/if}
          <div class="hero-shade"></div>
          <div class="hero-in">
            <div class="cover round"><Cover src={artist.cover} size={200} round alt="" /></div>
            <div class="hero-meta">
              <div class="k">Artist</div>
              <h1>{artist.name}</h1>
              {#if artist.subscribers}
                <div class="sub">{fmtCount(artist.subscribers)} subscribers</div>
              {/if}
              <div class="hero-acts">
                <button class="btn" type="button" onclick={() => playTracks(artist!.tracks, 0)}><Hi icon={PlayIcon} size={18} /> Play</button>
                <button class="btn ghost" type="button" onclick={() => { api.setShuffle(true); playTracks([...artist!.tracks].sort(() => Math.random() - 0.5), 0); }}><Hi icon={ShuffleIcon} size={18} /> Shuffle</button>
                {#if artist.radio_id}
                  <button class="btn ghost" type="button" onclick={() => api.artistRadio(artist!.radio_id!)}><Hi icon={Radio02Icon} size={18} /> Radio</button>
                {/if}
                {#if auth.signed_in}
                  <button class="btn ghost {artist.subscribed ? 'on' : ''}" type="button" onclick={async () => { const on = !artist!.subscribed; try { await api.subscribe(artist!.id, on); artist = { ...artist!, subscribed: on }; } catch (e) { err = String(e); } }}>
                    {artist.subscribed ? 'Subscribed' : 'Subscribe'}
                  </button>
                {/if}
              </div>
            </div>
          </div>
        </div>
        {#if artist.tracks.length}
          <h2 class="shelf-title" style="margin-bottom:12px">Top songs</h2>
          <TrackTable tracks={artist.tracks} currentId={player.track?.id} playing={player.playing} showAlbum={true}
            onPlay={(t, i) => playTracks(artist!.tracks, i)}
            onArtist={(id, n) => goArtist(id, n)}
            onAlbum={(id) => go({ id: 'album', albumId: id })}
            onContext={(e, t) => contextTrack(e, t)} />
        {/if}
        {#if artist.albums.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">Albums</h2>
          <div class="row">
            {#each artist.albums as a}
              <button class="card" type="button" onclick={() => go({ id: 'album', albumId: a.id })}>
                <div class="art"><Cover src={a.cover} size={152} alt="" /></div>
                <div class="c-a"><div class="title">{a.name}</div><div class="sub">{a.year || a.album_type || ''}</div></div>
              </button>
            {/each}
          </div>
        {/if}
        {#if artist.singles.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">Singles</h2>
          <div class="row">
            {#each artist.singles as a}
              <button class="card" type="button" onclick={() => go({ id: 'album', albumId: a.id })}>
                <div class="art"><Cover src={a.cover} size={152} alt="" /></div>
                <div class="c-a"><div class="title">{a.name}</div><div class="sub">{a.year || a.album_type || ''}</div></div>
              </button>
            {/each}
          </div>
        {/if}
        {#if artist.videos.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">Videos</h2>
          <TrackTable tracks={artist.videos} currentId={player.track?.id} playing={player.playing} showAlbum={false}
            onPlay={(t, i) => playTracks(artist!.videos, i)}
            onArtist={(id, n) => goArtist(id, n)}
            onAlbum={(id) => go({ id: 'album', albumId: id })}
            onContext={(e, t) => contextTrack(e, t)} />
        {/if}
        {#if artist.playlists.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">Featured</h2>
          <div class="row">
            {#each artist.playlists as p}
              <button class="card" type="button" onclick={() => go({ id: 'playlist', playlistId: p.id })}>
                <div class="art"><Cover src={p.cover} size={152} alt="" /></div>
                <div class="c-a"><div class="title">{p.name}</div></div>
              </button>
            {/each}
          </div>
        {/if}
        {#if artist.similar.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">Similar artists</h2>
          <div class="row">
            {#each artist.similar as a}
              <button class="card artist" type="button" onclick={() => go({ id: 'artist', artistId: a.id })}>
                <div class="art"><Cover src={a.cover} size={132} round alt="" /></div>
                <div class="c-a"><div class="title">{a.name}</div></div>
              </button>
            {/each}
          </div>
        {/if}

      {:else if (view.id === 'album' || view.id === 'playlist') && collection}
        <div class="hero" style="min-height:260px;background:var(--bg-elev)">
          <div class="hero-shade"></div>
          <div class="hero-in">
            <div class="cover"><Cover src={collection.cover} size={200} alt="" /></div>
            <div class="hero-meta">
              <div class="k">{collection.album_type || (collection.kind === 'album' ? 'Album' : 'Playlist')}</div>
              <h1>{collection.name}</h1>
              <div class="sub">
                {collection.author || artistsLine(collection.artists)}
                {#if collection.year} · {collection.year}{/if}
                {#if collection.track_count} · {collection.track_count}{/if}
              </div>
              <div class="hero-acts">
                <button class="btn" type="button" onclick={() => playTracks(collection!.tracks, 0)}><Hi icon={PlayIcon} size={18} /> Play</button>
                <button class="btn ghost" type="button" onclick={() => { api.setShuffle(true); playTracks([...collection!.tracks].sort(() => Math.random() - 0.5), 0); }}><Hi icon={ShuffleIcon} size={18} /> Shuffle</button>
                <button class="btn ghost" type="button" onclick={() => collection!.tracks[0] && api.radio(collection!.tracks[0].id)}><Hi icon={Radio02Icon} size={18} /> Radio</button>
                {#if auth.signed_in}
                  <button class="btn ghost {collection.saved ? 'on' : ''}" type="button" onclick={async () => { const s = !collection!.saved; try { await api.librarySave(collection!.playlist_id || collection!.id, s); collection = { ...collection!, saved: s }; } catch (e) { err = String(e); } }}>
                    <Hi icon={Bookmark02Icon} size={18} /> {collection.saved ? 'Saved' : 'Save'}
                  </button>
                {/if}
              </div>
            </div>
          </div>
        </div>
        <TrackTable tracks={collection.tracks} currentId={player.track?.id} playing={player.playing} showAlbum={collection.kind === 'playlist'} fallbackCover={collection.cover}
          onPlay={(t, i) => playTracks(collection!.tracks, i)}
          onArtist={(id, n) => goArtist(id, n)}
          onAlbum={(id) => go({ id: 'album', albumId: id })}
          onContext={(e, t) => contextTrack(e, t)} />
        {#if collection.related.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">Related</h2>
          <div class="row">
            {#each collection.related as p}
              <button class="card" type="button" onclick={() => go({ id: collection!.kind === 'album' ? 'album' : 'playlist', albumId: p.id, playlistId: p.id } as View)}>
                <div class="art"><Cover src={p.cover} size={152} alt="" /></div>
                <div class="c-a"><div class="title">{p.name}</div></div>
              </button>
            {/each}
          </div>
        {/if}

      {:else if view.id === 'genre'}
        <h1 class="page-title">{genre?.name || (view as { title?: string }).title || ''}</h1>
        {#each genre?.sections ?? [] as s}
          <ShelfRow shelf={s} onPlayItems={playTracks} onOpen={openItem} onContext={contextItem} />
        {/each}

      {:else if view.id === 'settings'}
        <h1 class="page-title">Settings</h1>
        <div class="set">
          <div class="set-row">
            <span class="lb"><Hi icon={MusicNote01Icon} size={20} /> Audio quality</span>
            <select bind:value={settings.quality} onchange={() => api.setSettings(settings)}>
              <option value="high">High</option>
              <option value="medium">Medium</option>
              <option value="low">Low</option>
            </select>
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={LanguageCircleIcon} size={20} /> Language</span>
            <select bind:value={settings.language} onchange={() => api.setSettings(settings)}>
              <option value="en">English</option>
              <option value="ro">Română</option>
              <option value="de">Deutsch</option>
              <option value="fr">Français</option>
              <option value="es">Español</option>
              <option value="pt">Português</option>
              <option value="it">Italiano</option>
              <option value="ja">日本語</option>
              <option value="ko">한국어</option>
              <option value="zh-CN">中文</option>
            </select>
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={Globe02Icon} size={20} /> Region</span>
            <select bind:value={settings.region} onchange={() => { api.setSettings(settings); home = null; if (view.id === 'home') loadHome(); }}>
              {#each ['US','RO','GB','DE','FR','ES','IT','CA','AU','BR','JP','KR','IN','MX','NL','SE','PL'] as r}
                <option value={r}>{r}</option>
              {/each}
            </select>
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={Notification01Icon} size={20} /> Notifications</span>
            <input type="checkbox" bind:checked={settings.notifications} onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={Radio02Icon} size={20} /> Autoplay</span>
            <input type="checkbox" bind:checked={settings.autoplay} onchange={() => api.setSettings(settings)} />
          </div>
          {#if auth.signed_in}
            <div class="set-row">
              <span class="lb"><Hi icon={Logout03Icon} size={20} /> {auth.account?.name || 'Account'}</span>
              <button class="btn ghost" type="button" onclick={async () => { await api.signOut(); auth = { signed_in: false, mock: false, account: null, store: null }; library = null; }}>Sign out</button>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </main>

  {#if panel}
    <aside class="panel">
      <div class="panel-h">
        <h2>{{ queue: 'Up next', lyrics: 'Lyrics', related: 'Related' }[panel]}</h2>
        <IconBtn icon={Cancel01Icon} onclick={() => (panel = null)} title="Close" />
      </div>
      <div class="ptabs">
        <button class="ptab {panel === 'queue' ? 'on' : ''}" type="button" onclick={() => openPanel('queue')}>Queue</button>
        <button class="ptab {panel === 'lyrics' ? 'on' : ''}" type="button" onclick={() => openPanel('lyrics')}>Lyrics</button>
        <button class="ptab {panel === 'related' ? 'on' : ''}" type="button" onclick={() => openPanel('related')}>Related</button>
      </div>
      <div class="panel-b">
        {#if panel === 'queue'}
          {#each player.queue as t, i}
            <button class="q-item {i === player.index ? 'on' : ''}" type="button" onclick={() => playTracks(player.queue, i)} oncontextmenu={(e) => contextTrack(e, t)}>
              <Cover src={t.cover} size={40} alt="" />
              <div class="tx">
                <div class="tn">{t.title}</div>
                <div class="ta">{artistsLine(t.artists)}</div>
              </div>
              {#if i !== player.index}
                <IconBtn icon={Cancel01Icon} size={16} onclick={(e) => { e.stopPropagation(); api.queueRemove(i); }} title="Remove" />
              {/if}
            </button>
          {/each}
        {:else if panel === 'lyrics'}
          <div class="lyrics">
            {#if lyrics && lyrics.lines.length}
              {#each lyrics.lines as ln, i}
                <button type="button" class="ly {i === activeLyric ? 'on' : ''}" data-i={i}
                  onclick={() => ln.start_ms != null && api.seek(ln.start_ms / 1000)}
                >{ln.text}</button>
              {/each}
              {#if lyrics.source}<div class="ly-src">{lyrics.source}</div>{/if}
            {:else}
              <div class="empty"></div>
            {/if}
          </div>
        {:else if panel === 'related' && related}
          {#if related.tracks.length}
            {#each related.tracks as t, i}
              <button class="q-item" type="button" onclick={() => playTracks(related!.tracks, i)} oncontextmenu={(e) => contextTrack(e, t)}>
                <Cover src={t.cover} size={40} alt="" />
                <div class="tx"><div class="tn">{t.title}</div><div class="ta">{artistsLine(t.artists)}</div></div>
              </button>
            {/each}
          {/if}
          {#if related.artists.length}
            <div class="shelf-title" style="font-size:16px;margin:16px 8px 8px">Artists</div>
            <div class="row">
              {#each related.artists as a}
                <button class="card artist" type="button" onclick={() => go({ id: 'artist', artistId: a.id })}>
                  <div class="art"><Cover src={a.cover} size={132} round alt="" /></div>
                  <div class="c-a"><div class="title">{a.name}</div></div>
                </button>
              {/each}
            </div>
          {/if}
        {/if}
      </div>
    </aside>
  {/if}

  {#if expanded && player.track}
    <div class="np">
      <Cover src={player.track.cover} width={1400} height={900} class="np-bg" alt="" />
      <div class="np-shade"></div>
      <button class="ibtn lg" type="button" style="position:absolute;top:16px;right:16px;z-index:2" onclick={() => (expanded = false)} title="Close">
        <Hi icon={Cancel01Icon} size={22} />
      </button>
      <div class="np-in">
        <div class="np-art"><Cover src={player.track.cover} size={360} alt="" /></div>
        <div class="np-meta">
          <h1>{player.track.title}</h1>
          <div class="ar">
            {#each player.track.artists as a, k}
              {#if k > 0}<span>, </span>{/if}
              <button class="linkish" type="button" onclick={() => goArtist(player.track!.artist_ids[k], a)}>{a}</button>
            {/each}
          </div>
        </div>
      </div>
    </div>
  {/if}

  <footer class="player">
    <div class="player-in">
      <div class="now">
        {#if player.track}
          <button class="cov" type="button" onclick={() => (expanded = !expanded)} title="Now playing">
            <Cover src={player.track.cover} size={64} alt="" />
          </button>
          <div class="meta">
            <div class="title">{player.track.title}</div>
            <div class="artist">
              {#each player.track.artists as a, k}
                {#if k > 0}<span>, </span>{/if}
                <button type="button" onclick={() => goArtist(player.track!.artist_ids[k], a)}>{a}</button>
              {/each}
            </div>
          </div>
          <div class="likes">
            <IconBtn icon={FavouriteIcon} size={22} on={player.liked} onclick={toggleLike} title="Like" />
            <IconBtn icon={ThumbsDownIcon} size={22} on={player.disliked} danger onclick={toggleDislike} title="Dislike" />
          </div>
        {/if}
      </div>
      <div class="ctrls">
        <div class="row-c">
          <IconBtn icon={ShuffleIcon} size={22} on={player.shuffle} onclick={() => api.setShuffle(!player.shuffle)} title="Shuffle" btnSize="lg" />
          <IconBtn icon={PreviousIcon} size={26} btnSize="lg" onclick={() => api.prev()} title="Previous" />
          <IconBtn
            icon={PlayIcon}
            altIcon={PauseIcon}
            showAlt={player.playing}
            size={28}
            btnSize="xl"
            onclick={() => api.toggle()}
            title={player.playing ? 'Pause' : 'Play'}
            disabled={!player.track && !player.loading}
          />
          <IconBtn icon={NextIcon} size={26} btnSize="lg" onclick={() => api.next()} title="Next" />
          <IconBtn
            icon={RepeatIcon}
            altIcon={RepeatOne01Icon}
            showAlt={player.repeat === 'one'}
            size={22}
            on={player.repeat !== 'off'}
            btnSize="lg"
            onclick={() => api.cycleRepeat()}
            title="Repeat"
          />
        </div>
        <div class="seek">
          <span class="t">{fmtTime(scrub ?? player.position)}</span>
          <Slider
            value={player.position}
            max={player.duration || 0}
            disabled={!player.track || !(player.duration > 0)}
            showTip
            format={fmtTime}
            onpreview={previewSeek}
            onchange={commitSeek}
          />
          <span class="t end">{fmtTime(player.duration)}</span>
        </div>
      </div>
      <div class="right-c">
        {#if player.error}<span class="err">{player.error}</span>{/if}
        {#if player.loading}<div class="spin"></div>{/if}
        <IconBtn icon={MicVocalIcon} size={22} on={panel === 'lyrics'} onclick={() => openPanel('lyrics')} title="Lyrics" btnSize="lg" />
        <IconBtn icon={Queue02Icon} size={22} on={panel === 'queue'} onclick={() => openPanel('queue')} title="Queue" btnSize="lg" />
        <IconBtn icon={Radio02Icon} size={22} onclick={() => player.track && api.radio(player.track.id)} title="Radio" btnSize="lg" />
        <div class="vol">
          <Hi icon={player.volume < 0.01 ? VolumeMute01Icon : VolumeHighIcon} size={20} />
          <Slider
            value={player.volume}
            max={1}
            live
            wheel
            oninput={(v) => api.setVolume(v)}
          />
        </div>
      </div>
    </div>
  </footer>
</div>

{#if ctx}
  <ContextMenu
    x={ctx.x}
    y={ctx.y}
    track={ctx.track}
    signedIn={auth.signed_in}
    playlists={library?.playlists ?? []}
    onPlay={() => ctx?.track && playOne(ctx.track)}
    onPlayNext={() => ctx?.track && api.queueAdd([ctx.track], true)}
    onQueue={() => ctx?.track && api.queueAdd([ctx.track], false)}
    onRadio={() => ctx?.track && api.radio(ctx.track.id)}
    onLike={() => ctx?.track && api.like(ctx.track.id, 'like')}
    onDislike={() => ctx?.track && api.like(ctx.track.id, 'dislike')}
    onArtist={() => goArtist(ctx?.artistId || ctx?.track?.artist_ids?.[0] || '', ctx?.track?.artists?.[0])}
    onAlbum={() => (ctx?.albumId || ctx?.track?.album_id) && go({ id: 'album', albumId: ctx!.albumId || ctx!.track!.album_id! })}
    onShare={() => share(ctx?.track)}
    onAddTo={(id) => addToPlaylist(id)}
    onSave={() => (ctx?.playlistId || ctx?.albumId) && api.librarySave(ctx!.playlistId || ctx!.albumId!, true)}
    onClose={() => (ctx = null)}
  />
{/if}
