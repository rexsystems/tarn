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
  import { paintAccent } from './lib/contrast';
  import Cover from './lib/components/Cover.svelte';
  import Hi from './lib/components/Hi.svelte';
  import IconBtn from './lib/components/IconBtn.svelte';
  import ShelfRow from './lib/components/ShelfRow.svelte';
  import TrackTable from './lib/components/TrackTable.svelte';
  import ContextMenu from './lib/components/ContextMenu.svelte';
  import Slider from './lib/components/Slider.svelte';
  import TitleBar from './lib/components/TitleBar.svelte';
  import Dropdown from './lib/components/Dropdown.svelte';
  import Toggle from './lib/components/Toggle.svelte';
  import Skeleton from './lib/components/Skeleton.svelte';
  import SearchBar from './lib/components/SearchBar.svelte';
  import Toast from './lib/components/Toast.svelte';
  import UpdatePrompt from './lib/components/UpdatePrompt.svelte';
  import { fail, authPrompt, toast } from './lib/toast';
  import { friendlyError, isAuthError } from './lib/errors';
  import { localLikes, isLocalLiked, setLocalLiked, localRecents, recordRecent } from './lib/local';
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
  let updateCheck = $state(0);
  let appVersion = $state('0.1.0');
  let settings = $state<SettingsT>({ quality: 'high', language: 'en', region: 'US', notifications: false, autoplay: true, cache_max_mb: 1024, crossfade: true, crossfade_seconds: 4, album_continuity: true });
  let player = $state<PlayerState>({
    track: null, playing: false, loading: false, buffering: false, buffered: 0,
    position: 0, duration: 0,
    volume: 0.85, queue: [], index: 0, error: null, accent: null,
    shuffle: false, repeat: 'off', liked: false, disliked: false, autoplay: true,
  });
  let loading = $state(false);
  let libLoading = $state(false);
  let chip = $state<string | null>(null);
  let mainEl: HTMLElement | undefined = $state();
  let likes = $state<Track[]>(localLikes());
  let recents = $state<Track[]>(localRecents());
  type HistEntry = { view: View; scroll: number; searchQ: string; results: SearchResults | null; searchTab: typeof searchTab };
  let backStack = $state<HistEntry[]>([]);
  let fwdStack = $state<HistEntry[]>([]);
  let navLock = false;
  let panel = $state<null | 'queue' | 'lyrics' | 'related'>(null);
  let expanded = $state(false);
  let npShow = $state(true);
  let npHoverChrome = $state(false);
  let npHideTimer: ReturnType<typeof setTimeout> | undefined;
  let scrub = $state<number | null>(null);
  let lyrics = $state<Lyrics | null>(null);
  let related = $state<Related | null>(null);
  let ctx = $state<CtxTarget | null>(null);
  let addTitle = $state('');
  let cacheUsed = $state(0);
  let cacheMax = $state(1024 * 1024 * 1024);

  function homeCacheKey(account?: string | null): string {
    return `tarn.home.v2.${account || 'anon'}`;
  }
  function clearHomeCaches() {
    try {
      const drop: string[] = [];
      for (let i = 0; i < localStorage.length; i++) {
        const k = localStorage.key(i);
        if (k && (k.startsWith('tarn.home.') || k === 'tarn.home.v1')) drop.push(k);
      }
      for (const k of drop) localStorage.removeItem(k);
    } catch {}
  }
  function readHomeCache(account?: string | null): Home | null {
    try {
      const raw = localStorage.getItem(homeCacheKey(account));
      if (!raw) return null;
      const v = JSON.parse(raw);
      const h = (v && v.home) ? v.home as Home : v as Home;
      // Never serve a logged-out (or other-account) feed when signed in.
      if (account && h && h.signed_in === false) return null;
      return h;
    } catch { return null; }
  }
  function writeHomeCache(h: Home, account?: string | null) {
    try {
      const key = homeCacheKey(account ?? (auth.signed_in ? (auth.account?.name ?? 'user') : 'anon'));
      localStorage.setItem(key, JSON.stringify({ at: Date.now(), account: key, home: h }));
    } catch {}
  }
  function dedupeTracks(list: Track[]): Track[] {
    const seen = new Set<string>();
    return list.filter((t) => t.id && !seen.has(t.id) && (seen.add(t.id), true));
  }

  $effect(() => {
    if (player.accent) {
      const [r, g, b] = player.accent;
      const paint = paintAccent(r, g, b);
      const root = document.documentElement.style;
      root.setProperty('--accent-rgb', paint.rgb);
      root.setProperty('--accent', paint.css);
      root.setProperty('--accent-dim', paint.dim);
      root.setProperty('--accent-fg', paint.fg);
      root.setProperty('--accent-fg-rgb', paint.fgRgb);
    }
  });

  onMount(() => {
    const unsubs: Array<() => void> = [];
    (async () => {
      /* home cache is account-keyed; filled after auth() below */
      likes = localLikes();
      recents = localRecents();
      try {
        player = await api.state();
        unsubs.push(await listen<string>('toast', (e) => {
          if (e.payload) toast(String(e.payload), 'info', 3600);
        }));
        auth = await api.auth();
        settings = await api.settings();
        const acct = auth.signed_in ? (auth.account?.name ?? 'user') : 'anon';
        if (auth.signed_in) {
          // Drop any leftover logged-out snapshot so we never flash it.
          try { localStorage.removeItem(homeCacheKey('anon')); } catch {}
        }
        home = readHomeCache(acct);
      } catch {}
      if (!auth.signed_in && player.track && isLocalLiked(player.track.id)) {
        player.liked = true;
      }
      try {
        const sess = await api.savedSession();
        if (sess?.search_q) searchQ = sess.search_q;
        if (sess?.view && sess.view.id) {
          await go(sess.view, { replace: true, scroll: sess.scroll ?? 0 });
          if (sess.view.id === 'search' && sess.search_q) {
            await runSearch(sess.search_q, { replace: true });
          }
        }
      } catch {}
      if (view.id === 'home') await loadHome();
      if (auth.signed_in) loadLibrary();
    })();
    import('@tauri-apps/api/app').then((m) => m.getVersion()).then((v) => { appVersion = v; }).catch(() => {});
    listen<PlayerState>('player', (e) => {
      const next = e.payload;
      if (!auth.signed_in && next.track && isLocalLiked(next.track.id)) {
        next.liked = true;
        next.disliked = false;
      }
      if (next.error) {
        if (isAuthError(next.error)) {
          next.error = null;
          authPrompt('expired');
        } else {
          next.error = friendlyError(next.error, "Can't play this track");
        }
      }
      player = next;
    }).then((u) => unsubs.push(u));
    listen('auth', () => {
      api.auth().then((a) => {
        const was = auth.signed_in;
        auth = a;
        if (a.signed_in !== was) {
          clearHomeCaches();
          home = null;
          explore = null;
        }
        if (a.signed_in) {
          loadLibrary();
          loadHome();
          if (view.id === 'explore') loadExplore();
          for (const t of localLikes()) {
            api.like(t.id, 'like').catch(() => {});
          }
        } else if (was) {
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
      if ((e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) || e.key === 'BrowserBack' || e.key === 'BrowserForward') {
        e.preventDefault();
        if (e.key === 'ArrowLeft' || e.key === 'BrowserBack') goBack();
        else goFwd();
        return;
      }
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

    const mouseNav = (e: MouseEvent) => {
      if (e.button === 3 || e.button === 4) {
        e.preventDefault();
        if (e.type !== 'mousedown') {
          if (e.button === 3) goBack();
          else goFwd();
        }
      }
    };
    window.addEventListener('mousedown', mouseNav, true);
    window.addEventListener('mouseup', mouseNav, true);
    window.addEventListener('auxclick', mouseNav, true);

    const tick = setInterval(() => rememberUi(), 3000);
    const onHide = () => {
      rememberUi();
      api.saveSessionNow().catch(() => {});
    };
    window.addEventListener('pagehide', onHide);
    window.addEventListener('beforeunload', onHide);

    unsubs.push(() => {
      window.removeEventListener('mousedown', mouseNav, true);
      window.removeEventListener('mouseup', mouseNav, true);
      window.removeEventListener('auxclick', mouseNav, true);
      window.removeEventListener('pagehide', onHide);
      window.removeEventListener('beforeunload', onHide);
      clearInterval(tick);
    });

    return () => {
      unsubs.forEach((u) => u());
      window.removeEventListener('keydown', keys);
    };
  });

  function viewKey(v: View, q = searchQ): string {
    return JSON.stringify(v) + (v.id === 'search' ? `::${q}` : '');
  }

  function snapshot(): HistEntry {
    return {
      view,
      scroll: mainEl?.scrollTop ?? 0,
      searchQ,
      results,
      searchTab,
    };
  }

  function rememberUi() {
    api.rememberUi({
      view,
      search_q: searchQ || null,
      scroll: mainEl?.scrollTop ?? 0,
    }).catch(() => {});
  }

  function fillView(v: View) {
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

  function applyEntry(e: HistEntry) {
    navLock = true;
    view = e.view;
    searchQ = e.searchQ;
    results = e.results;
    searchTab = e.searchTab;
    fillView(e.view);
    const top = e.scroll;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (mainEl) mainEl.scrollTop = top;
        navLock = false;
      });
    });
    rememberUi();
  }

  function go(v: View, opts?: { replace?: boolean; scroll?: number }) {
    if (!opts?.replace && !navLock && viewKey(v) !== viewKey(view)) {
      backStack = [...backStack, snapshot()];
      fwdStack = [];
    }
    view = v;
    ctx = null;
    fillView(v);
    const top = opts?.scroll ?? 0;
    requestAnimationFrame(() => {
      if (mainEl) mainEl.scrollTop = top;
    });
    rememberUi();
  }

  function goBack() {
    const prev = backStack[backStack.length - 1];
    if (!prev) return;
    backStack = backStack.slice(0, -1);
    fwdStack = [...fwdStack, snapshot()];
    applyEntry(prev);
  }

  function goFwd() {
    const next = fwdStack[fwdStack.length - 1];
    if (!next) return;
    fwdStack = fwdStack.slice(0, -1);
    backStack = [...backStack, snapshot()];
    applyEntry(next);
  }

  function needAccount(reason: 'like' | 'dislike' | 'save' | 'playlist' | 'subscribe' | 'expired' | 'continue'): boolean {
    if (auth.signed_in) return false;
    authPrompt(reason);
    return true;
  }

  async function copyDiagnostics() {
    try {
      const text = await api.diagnostics();
      await navigator.clipboard.writeText(text);
      toast('Diagnostics copied', 'info', 2500);
    } catch (e) {
      fail(e, "Couldn't copy diagnostics");
    }
  }
  async function loadHome(params?: string | null) {
    const quiet = !params && !!home;
    if (!quiet) loading = true;
    try {
      const h = await api.home(params);
      home = h;
      if (!params) writeHomeCache(h, auth.signed_in ? (auth.account?.name ?? 'user') : 'anon');
    } catch (e) {
      if (!home) fail(e, "Couldn't load Home");
    } finally {
      loading = false;
    }
  }
  async function loadExplore() {
    loading = true;
    try { explore = await api.explore(); } catch (e) { fail(e, "Couldn't load Explore"); } finally { loading = false; }
  }
  async function loadArtist(id: string) {
    loading = true; artist = null;
    try { artist = await api.artist(id); } catch (e) { fail(e, "Couldn't load this artist"); } finally { loading = false; }
  }
  async function loadAlbum(id: string) {
    loading = true; collection = null;
    try { collection = await api.album(id); } catch (e) { fail(e, "Couldn't load this album"); } finally { loading = false; }
  }
  async function loadPlaylist(id: string) {
    loading = true; collection = null;
    try { collection = await api.playlist(id); } catch (e) { fail(e, "Couldn't load this playlist"); } finally { loading = false; }
  }
  async function loadGenre(id: string) {
    loading = true; genre = null;
    try { genre = await api.genre(id); } catch (e) { fail(e, "Couldn't load this page"); } finally { loading = false; }
  }
  async function loadCharts() {
    loading = true;
    try { charts = await api.charts(); } catch (e) { fail(e, "Couldn't load charts"); } finally { loading = false; }
  }
  async function loadReleases() {
    loading = true;
    try { releases = await api.newReleases(); } catch (e) { fail(e, "Couldn't load new releases"); } finally { loading = false; }
  }
  async function loadLibrary() {
    if (!auth.signed_in) {
      libLoading = false;
      return;
    }
    if (!library) libLoading = true;
    try {
      library = await api.library();
    } catch (e) {
      fail(e, "Couldn't load your library");
      if (!library) {
        library = { signed_in: auth.signed_in, mock: auth.mock, account: auth.account, playlists: [], liked: [], albums: [], artists: [], recents: [] };
      }
    } finally {
      libLoading = false;
    }
  }
  async function loadHistory() {
    loading = true;
    try { history = await api.history(); } catch (e) { fail(e, "Couldn't load history"); history = []; } finally { loading = false; }
  }

  async function runSearch(q?: string, opts?: { replace?: boolean }) {
    const qq = (q ?? searchQ).trim();
    if (!qq) return;
    searchQ = qq;
    loading = true;
    if (view.id !== 'search') go({ id: 'search' }, { replace: opts?.replace });
    else rememberUi();
    try {
      results = await api.searchAll(qq);
      searchTab = 'top';
    } catch (e) {
      fail(e, "Couldn't search");
    } finally {
      loading = false;
    }
  }

  async function playTracks(tracks: Track[], index: number) {
    if (!tracks.length) return;
    const t = tracks[index] ?? tracks[0];
    recents = recordRecent(t);
    try {
      await api.play(t, tracks, index);
    } catch (e) {
      fail(e, "Can't play this track");
    }
  }
  function playOne(t: Track) { playTracks([t, ...player.queue.filter((x) => x.id !== t.id)], 0); }

  async function startRadio(id?: string) {
    const vid = id || player.track?.id;
    if (!vid) return;
    try { await api.radio(vid); } catch (e) { fail(e, "Couldn't start radio"); }
  }

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
    } catch (e) { fail(e, "Can't play this"); }
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

  function applyLocalLike(track: Track, liked: boolean) {
    likes = setLocalLiked(track, liked);
    if (player.track?.id === track.id) {
      player.liked = liked;
      if (liked) player.disliked = false;
    }
  }

  async function likeTrack(track: Track | undefined, action: string) {
    if (!track?.id) return;
    if (!auth.signed_in) {
      if (action === 'like') applyLocalLike(track, true);
      else if (action === 'removelike') applyLocalLike(track, false);
      else if (action === 'dislike') applyLocalLike(track, false);
      authPrompt(action === 'dislike' ? 'dislike' : 'like');
      return;
    }
    try {
      const st = await api.like(track.id, action);
      player.liked = st.liked;
      player.disliked = st.disliked;
      if (player.track?.id === track.id) {
        /* already applied */
      }
      if (st.liked) likes = setLocalLiked(track, true);
      else if (action !== 'dislike') likes = setLocalLiked(track, false);
    } catch (e) {
      fail(e, "Couldn't update this", action === 'dislike' ? 'dislike' : 'like');
    }
  }
  async function like(action: string) {
    const t = player.track;
    if (t) await likeTrack(t, action);
  }
  async function toggleLike() {
    const t = player.track;
    if (!t) return;
    await likeTrack(t, (player.liked || isLocalLiked(t.id)) ? 'removelike' : 'like');
  }
  async function toggleDislike() {
    await like(player.disliked ? 'removelike' : 'dislike');
  }

  async function openPanel(tab: 'queue' | 'lyrics' | 'related') {
    panel = panel === tab ? null : tab;
  }

  // Fetch lyrics/related once per track. A plain (non-reactive) last-id
  // is required: the player snapshot is replaced every 200 ms and a
  // naive $effect would refetch in a loop.
  let relatedFor = '';
  let lyricsFor = '';
  $effect(() => {
    const id = player.track?.id ?? '';
    const p = panel;
    if (p === 'related' && id && relatedFor !== id) {
      relatedFor = id;
      const req = id;
      api.related(id).then((r) => {
        if (relatedFor !== req) return;
        related = { ...r, tracks: dedupeTracks(r.tracks), versions: dedupeTracks(r.versions || []) };
      }).catch(() => {
        if (relatedFor === req) related = { tracks: [], versions: [], albums: [], artists: [], playlists: [] };
      });
    }
    if (p === 'lyrics' && id && lyricsFor !== id) {
      lyricsFor = id;
      const req = id;
      api.lyrics(id).then((l) => { if (lyricsFor === req) lyrics = l; }).catch(() => {
        if (lyricsFor === req) lyrics = { source: null, lines: [], synced: false };
      });
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
    if (needAccount('playlist')) return;
    if (!pid) {
      const title = prompt('Playlist name');
      if (!title) return;
      try {
        const id = await api.playlistCreate(title);
        await api.playlistAdd(id, vid);
        loadLibrary();
      } catch (e) { fail(e, "Couldn't create this playlist", 'playlist'); }
      return;
    }
    try { await api.playlistAdd(pid, vid); } catch (e) { fail(e, "Couldn't add to this playlist", 'playlist'); }
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

  function fmtCache(n: number) {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(n >= 10 * 1024 * 1024 ? 0 : 1)} MB`;
    return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }
  async function refreshCacheStats() {
    try {
      const [used, max] = await api.cacheStats();
      cacheUsed = used;
      cacheMax = max;
    } catch {}
  }
  $effect(() => {
    if (view.id === 'settings') refreshCacheStats();
  });

  let navOn = (id: string) => view.id === id;

  function armNpHide() {
    npShow = true;
    if (npHideTimer) clearTimeout(npHideTimer);
    if (expanded && !npHoverChrome) {
      npHideTimer = setTimeout(() => {
        if (expanded && !npHoverChrome) npShow = false;
      }, 2500);
    }
  }

  function onAppMove() {
    if (expanded) armNpHide();
  }

  $effect(() => {
    if (expanded) armNpHide();
    else {
      npShow = true;
      if (npHideTimer) clearTimeout(npHideTimer);
    }
  });
</script>

<div
  class="app"
  class:np-open={expanded}
  class:np-idle={expanded && !npShow}
  style="--player: 112px; --side: 248px; --titlebar: 36px;"
  role="application"
  onmousemove={onAppMove}
>
  <div
    class="tb-wrap"
    role="presentation"
    onpointerenter={() => { if (expanded) { npHoverChrome = true; armNpHide(); } }}
    onpointerleave={() => { if (expanded) { npHoverChrome = false; armNpHide(); } }}
  >
    <TitleBar canBack={backStack.length > 0} canFwd={fwdStack.length > 0} onBack={goBack} onFwd={goFwd} />
  </div>
  <aside class="sidebar" class:is-hidden={expanded}>
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

  <main class="main {panel ? 'with-panel' : ''}" class:is-hidden={expanded} bind:this={mainEl}>
    <div class="page">

      {#if view.id === 'home'}
        {#if home?.chips?.length}
          <div class="chips">
            {#each home.chips as c}
              <button class="chip {chip === c.title || c.selected ? 'on' : ''}" type="button" onclick={() => openChip(c)}>{c.title}</button>
            {/each}
          </div>
        {/if}
        {#if loading && !home}<Skeleton kind="home" />{/if}
        {#each home?.shelves ?? [] as s}
          <ShelfRow shelf={s} onPlayItems={playTracks} onOpen={openItem} onContext={contextItem} />
        {/each}

      {:else if view.id === 'explore'}
        <SearchBar
          bind:value={searchQ}
          onSubmit={(q) => runSearch(q)}
          onPlay={(t, queue) => playTracks(queue, Math.max(0, queue.findIndex((x) => x.id === t.id)))}
          onOpen={openItem}
        />
        <h1 class="page-title">Explore</h1>
        <div class="explore-links">
          <button type="button" onclick={() => go({ id: 'new-releases' })}><Hi icon={Album02Icon} size={24} /> New releases</button>
          <button type="button" onclick={() => go({ id: 'charts' })}><Hi icon={MusicNote01Icon} size={24} /> Charts</button>
          <button type="button" onclick={() => document.getElementById('moods')?.scrollIntoView({ behavior: 'smooth' })}><Hi icon={Compass01Icon} size={24} /> Moods & genres</button>
        </div>
        {#if loading && !explore}<Skeleton kind="explore" />{/if}
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
        {#if loading && !(view.id === 'charts' ? charts : releases)}<Skeleton kind="home" />{/if}
        {#each (view.id === 'charts' ? charts : releases)?.shelves ?? [] as s}
          <ShelfRow shelf={s} onPlayItems={playTracks} onOpen={openItem} onContext={contextItem} />
        {/each}

      {:else if view.id === 'search'}
        <SearchBar
          bind:value={searchQ}
          autofocus
          onSubmit={(q) => runSearch(q)}
          onPlay={(t, queue) => playTracks(queue, Math.max(0, queue.findIndex((x) => x.id === t.id)))}
          onOpen={openItem}
        />
        {#if loading && !results}<Skeleton kind="search" />{/if}
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
              <div class="top-art">
                <Cover src={top.cover} size={160} round={top.kind === 'artist'} alt="" />
                {#if top.kind === 'track' || top.kind === 'video'}
                  <span class="play-ov"><Hi icon={PlayIcon} size={40} /></span>
                {/if}
              </div>
              <div>
                <div class="k">{top.kind}</div>
                <div class="ttl">{top.title}</div>
                <div class="sub">{top.subtitle || artistsLine(top.artists)}</div>
                <div class="acts">
                  {#if top.kind === 'track' || top.kind === 'video'}
                    <button class="btn" type="button" onclick={(e) => { e.stopPropagation(); playOne(itemToTrack(top)); }}><Hi icon={PlayIcon} size={18} /> Play</button>
                    <button class="btn ghost" type="button" onclick={(e) => { e.stopPropagation(); playTopShuffled(top); }}><Hi icon={ShuffleIcon} size={18} /> Shuffle</button>
                    <button class="btn ghost" type="button" onclick={(e) => { e.stopPropagation(); startRadio(top.id); }}><Hi icon={Radio02Icon} size={18} /> Radio</button>
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
          <div class="lib-guest">
            <img src="/logo.png" width="80" height="80" alt="" />
            <p>Sign in to see your playlists, likes and albums</p>
            <button class="btn" type="button" onclick={() => api.openLogin()}>Sign in</button>
          </div>
          {#if recents.length}
            <h2 class="shelf-title" style="margin:12px 0 12px">Recently played</h2>
            <TrackTable
              tracks={recents}
              currentId={player.track?.id}
              playing={player.playing}
              onPlay={(t, i) => playTracks(recents, i)}
              onArtist={(id, n) => goArtist(id, n)}
              onAlbum={(id) => go({ id: 'album', albumId: id })}
              onContext={(e, t) => contextTrack(e, t)}
            />
          {/if}
          {#if likes.length}
            <h2 class="shelf-title" style="margin:28px 0 12px">Liked songs</h2>
            <TrackTable
              tracks={likes}
              currentId={player.track?.id}
              playing={player.playing}
              onPlay={(t, i) => playTracks(likes, i)}
              onArtist={(id, n) => goArtist(id, n)}
              onAlbum={(id) => go({ id: 'album', albumId: id })}
              onContext={(e, t) => contextTrack(e, t)}
            />
          {/if}
        {:else}
          {#if libLoading && !library}
            <Skeleton kind="page" />
          {/if}
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

      {:else if view.id === 'artist'}
        {#if loading && !artist}<Skeleton kind="artist" />{/if}
        {#if artist}
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
                  <button class="btn ghost {artist.subscribed ? 'on' : ''}" type="button" onclick={async () => { if (needAccount('subscribe')) return; const on = !artist!.subscribed; try { await api.subscribe(artist!.id, on); artist = { ...artist!, subscribed: on }; } catch (e) { fail(e, "Couldn't subscribe", 'subscribe'); } }}>
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

        {/if}
      {:else if view.id === 'album' || view.id === 'playlist'}
        {#if loading && !collection}<Skeleton kind="album" />{/if}
        {#if collection}
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
                <button class="btn ghost" type="button" onclick={() => collection!.tracks[0] && startRadio(collection!.tracks[0].id)}><Hi icon={Radio02Icon} size={18} /> Radio</button>
                {#if auth.signed_in}
                  <button class="btn ghost {collection.saved ? 'on' : ''}" type="button" onclick={async () => { if (needAccount('save')) return; const s = !collection!.saved; try { await api.librarySave(collection!.playlist_id || collection!.id, s); collection = { ...collection!, saved: s }; } catch (e) { fail(e, "Couldn't save this", 'save'); } }}>
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

        {/if}
      {:else if view.id === 'genre'}
        {#if loading && !genre}<Skeleton kind="page" />{/if}
        <h1 class="page-title">{genre?.name || (view as { title?: string }).title || ''}</h1>
        {#each genre?.sections ?? [] as s}
          <ShelfRow shelf={s} onPlayItems={playTracks} onOpen={openItem} onContext={contextItem} />
        {/each}

      {:else if view.id === 'settings'}
        <h1 class="page-title">Settings</h1>
        <div class="set">
          <div class="set-row">
            <span class="lb"><Hi icon={MusicNote01Icon} size={20} /> Audio quality</span>
            <Dropdown bind:value={settings.quality} options={[
              { value: 'high', label: 'High' },
              { value: 'medium', label: 'Medium' },
              { value: 'low', label: 'Low' },
            ]} onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={LanguageCircleIcon} size={20} /> Language</span>
            <Dropdown bind:value={settings.language} options={[
              { value: 'en', label: 'English' },
              { value: 'ro', label: 'Română' },
              { value: 'de', label: 'Deutsch' },
              { value: 'fr', label: 'Français' },
              { value: 'es', label: 'Español' },
              { value: 'pt', label: 'Português' },
              { value: 'it', label: 'Italiano' },
              { value: 'ja', label: '日本語' },
              { value: 'ko', label: '한국어' },
              { value: 'zh-CN', label: '中文' },
            ]} onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={Globe02Icon} size={20} /> Region</span>
            <Dropdown bind:value={settings.region} options={['US','RO','GB','DE','FR','ES','IT','CA','AU','BR','JP','KR','IN','MX','NL','SE','PL'].map((r) => ({ value: r, label: r }))} onchange={() => { api.setSettings(settings); home = null; if (view.id === 'home') loadHome(); }} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={Notification01Icon} size={20} /> Notifications</span>
            <Toggle bind:checked={settings.notifications} label="Notifications" onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={Radio02Icon} size={20} /> Autoplay</span>
            <Toggle bind:checked={settings.autoplay} label="Autoplay" onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={MusicNote01Icon} size={20} /> Crossfade</span>
            <Toggle bind:checked={settings.crossfade} label="Crossfade" onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb">Crossfade duration
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{settings.crossfade_seconds.toFixed(1)}s</span>
            </span>
            <div style="width:200px">
              <Slider
                value={settings.crossfade_seconds}
                max={12}
                live={true}
                showTip={true}
                format={(v) => `${v.toFixed(1)}s`}
                oninput={(v) => { settings.crossfade_seconds = Math.round(v * 10) / 10; }}
                onchange={(v) => { settings.crossfade_seconds = Math.round(v * 10) / 10; api.setSettings(settings); }}
              />
            </div>
          </div>
          <div class="set-row">
            <span class="lb">Album continuity
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">No crossfade between tracks on the same album</span>
            </span>
            <Toggle bind:checked={settings.album_continuity} label="Album continuity" onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={Bookmark02Icon} size={20} /> Offline cache
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{fmtCache(cacheUsed)} of {fmtCache(cacheMax)}</span>
            </span>
            <Dropdown
              value={String(settings.cache_max_mb)}
              options={[
                { value: '0', label: 'Off' },
                { value: '256', label: '256 MB' },
                { value: '512', label: '512 MB' },
                { value: '1024', label: '1 GB' },
                { value: '2048', label: '2 GB' },
                { value: '4096', label: '4 GB' },
              ]}
              onchange={(v) => { settings.cache_max_mb = Number(v); api.setSettings(settings); refreshCacheStats(); }}
            />
          </div>
          <div class="set-row">
            <span class="lb">Updates
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Tarn {appVersion}</span>
            </span>
            <button class="upd-check" type="button" onclick={() => (updateCheck += 1)}>Check for updates</button>
          </div>
          <div class="set-row">
            <span class="lb">Diagnostics
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Auth, library, and recent logs — no cookie values.</span>
            </span>
            <button class="btn ghost" type="button" onclick={copyDiagnostics}>Copy diagnostics</button>
          </div>
          {#if auth.signed_in}
            <div class="set-row">
              <span class="lb"><Hi icon={Logout03Icon} size={20} /> {auth.account?.name || 'Account'}</span>
              <button class="btn ghost" type="button" onclick={async () => { await api.signOut(); auth = { signed_in: false, mock: false, account: null, store: null }; library = null; clearHomeCaches(); home = null; explore = null; if (view.id === 'home') loadHome(); if (view.id === 'explore') loadExplore(); }}>Sign out</button>
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
          {#each player.queue as t, i (`${i}:${t.id}`)}
            <button class="q-item {i === player.index ? 'on' : ''}" type="button" onclick={() => playTracks(player.queue, i)} oncontextmenu={(e) => contextTrack(e, t)} onmouseenter={() => t.id && api.prefetch(t.id)}>
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
            {#each related.tracks as t, i (t.id || i)}
              <button class="q-item" type="button" onclick={() => playTracks(related!.tracks, i)} oncontextmenu={(e) => contextTrack(e, t)}>
                <Cover src={t.cover} size={40} alt="" />
                <div class="tx"><div class="tn">{t.title}</div><div class="ta">{artistsLine(t.artists)}</div></div>
              </button>
            {/each}
          {/if}
          {#if related.artists.length}
            <div class="shelf-title" style="font-size:16px;margin:16px 8px 8px">Artists</div>
            <div class="row">
              {#each related.artists as a (a.id)}
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
      <button
        class="ibtn lg np-close"
        type="button"
        onclick={() => (expanded = false)}
        title="Close"
        onpointerenter={() => { npHoverChrome = true; armNpHide(); }}
        onpointerleave={() => { npHoverChrome = false; armNpHide(); }}
      >
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

  <footer
    class="player"
    onpointerenter={() => { if (expanded) { npHoverChrome = true; armNpHide(); } }}
    onpointerleave={() => { if (expanded) { npHoverChrome = false; armNpHide(); } }}
  >
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
            showAlt={player.playing && !player.buffering}
            size={28}
            btnSize="xl"
            spinning={player.buffering || (player.loading && !player.playing)}
            onclick={() => api.toggle()}
            title={player.buffering ? 'Buffering' : player.playing ? 'Pause' : 'Play'}
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
            buffered={player.buffered}
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
        {#if player.error}<span class="err">{friendlyError(player.error, "Can't play this track")}</span>{/if}
        {#if player.loading}<div class="spin"></div>{/if}
        <IconBtn icon={MicVocalIcon} size={22} on={panel === 'lyrics'} onclick={() => openPanel('lyrics')} title="Lyrics" btnSize="lg" />
        <IconBtn icon={Queue02Icon} size={22} on={panel === 'queue'} onclick={() => openPanel('queue')} title="Queue" btnSize="lg" />
        <IconBtn icon={Radio02Icon} size={22} onclick={() => startRadio()} title="Radio" btnSize="lg" />
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

<Toast />
<UpdatePrompt bind:request={updateCheck} />

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
    onRadio={() => ctx?.track && startRadio(ctx.track.id)}
    onLike={() => ctx?.track && likeTrack(ctx.track, 'like')}
    onDislike={() => ctx?.track && likeTrack(ctx.track, 'dislike')}
    onArtist={() => goArtist(ctx?.artistId || ctx?.track?.artist_ids?.[0] || '', ctx?.track?.artists?.[0])}
    onAlbum={() => (ctx?.albumId || ctx?.track?.album_id) && go({ id: 'album', albumId: ctx!.albumId || ctx!.track!.album_id! })}
    onShare={() => share(ctx?.track)}
    onAddTo={(id) => addToPlaylist(id)}
    onSave={() => {
      if (needAccount('save')) return;
      const id = ctx?.playlistId || ctx?.albumId;
      if (id) api.librarySave(id, true).catch((e) => fail(e, "Couldn't save this", 'save'));
    }}
    onClose={() => (ctx = null)}
  />
{/if}
