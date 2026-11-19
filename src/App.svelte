<script lang="ts">
  import { t as tr, setLocale, localeOptions } from './lib/i18n.svelte';
  import Stats from './lib/components/Stats.svelte';
  import SmartPlaylists from './lib/components/SmartPlaylists.svelte';
  import PhoneRemote from './lib/components/PhoneRemote.svelte';
  import ListenTogether from './lib/components/ListenTogether.svelte';
  import { jam } from './lib/jam.svelte';
  import Shortcuts from './lib/components/Shortcuts.svelte';
  import { appCombo, actionFor, appShortcuts } from './lib/shortcuts';
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { api } from './lib/api';
  import type {
    ArtistPage, AuthStatus, Collection, Explore, GenrePage, Home, Library,
    Lyrics, PlayerState, PlaylistCard, Related, SearchResults, Settings as SettingsT,
    Shelf, ShelfItem, Track, View, CtxTarget,
  } from './lib/types';
  import { coverFor, itemToTrack, preloadNowCover } from './lib/cover';
  import { artistsLine, fmtCount, fmtTime, shareUrl } from './lib/format';
  import type { MenuEntry, CollTarget } from './lib/types';
  import { paintAccent } from './lib/contrast';
  import Cover from './lib/components/Cover.svelte';
  import PlaylistCover from './lib/components/PlaylistCover.svelte';
  import Hi from './lib/components/Hi.svelte';
  import IconBtn from './lib/components/IconBtn.svelte';
  import ShelfRow from './lib/components/ShelfRow.svelte';
  import TrackTable from './lib/components/TrackTable.svelte';
  import ContextMenu from './lib/components/ContextMenu.svelte';
  import SleepTimer from './lib/components/SleepTimer.svelte';
  import ImportPlaylist from './lib/components/ImportPlaylist.svelte';
  import Scrobbling from './lib/components/Scrobbling.svelte';
  import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
  import { Scrobbler } from './lib/scrobble';
  import { tracksToCsv } from './lib/playlistIo';
  import QueueList from './lib/components/QueueList.svelte';
  import FilterBar from './lib/components/FilterBar.svelte';
  import { off, initOffline, refreshOffline, sourceInfo, fmtBytes } from './lib/offline.svelte';
  import { filterTracks, filterNamed } from './lib/filter';
  import Slider from './lib/components/Slider.svelte';
  import Visualizer from './lib/components/Visualizer.svelte';
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
  import {Home01Icon, Search01Icon, Compass01Icon, LibraryIcon, Settings02Icon, PlayIcon, PauseIcon, PreviousIcon, NextIcon, ShuffleIcon, RepeatIcon, RepeatOne01Icon, VolumeHighIcon, VolumeMute01Icon, FavouriteIcon, FavouriteFilledIcon, ThumbsDownIcon, Queue02Icon, Radio02Icon, Cancel01Icon, HistoryIcon, UserCircle02Icon, Logout03Icon, Login03Icon, MicVocalIcon, Bookmark02Icon, Notification01Icon, DiscordIcon, Globe02Icon, LanguageCircleIcon, Album02Icon, Playlist01Icon, MusicNote01Icon, SlidersHorizontalIcon, Download04Icon, ChartColumnIcon, Forward01Icon, WifiOff01Icon, CheckmarkCircle02Icon, Delete02Icon, MagicWand01Icon, PlayListAddIcon, Link01Icon, UserIcon, Add01Icon, ArrowRight01Icon, Share01Icon, MiniPlayerIcon, AudioWave01Icon, PaintBoardIcon} from './lib/icons';
  import { applyTheme, ACCENTS, hexRgb, type Theme } from './lib/theme';

  const isMac =
    typeof navigator !== 'undefined' &&
    (/Mac/i.test(navigator.platform) || /macOS|Macintosh/i.test(navigator.userAgent));
  const isPhone = typeof navigator !== 'undefined' && /Android|iPhone|iPad/i.test(navigator.userAgent);
  const EQ_LABELS = ['60', '230', '910', '3.6k', '14k'] as const;
  type EqBands = SettingsT['eq_bands'];
  const EQ_PRESETS = $derived<{ id: string; label: string; bands: EqBands }[]>([
    { id: 'flat', label: tr('Flat'), bands: [0, 0, 0, 0, 0] },
    { id: 'bass', label: tr('Bass Boost'), bands: [7.5, 4.5, 0, -1, 0] },
    { id: 'bass-cut', label: tr('Bass Cut'), bands: [-6, -3.5, 0, 1.5, 2] },
    { id: 'treble', label: tr('Treble Boost'), bands: [-1, 0, 1, 4.5, 7] },
    { id: 'vocal', label: tr('Vocal'), bands: [-3, -1, 4, 3.5, 1] },
    { id: 'rock', label: tr('Rock'), bands: [5, 2, -1.5, 2.5, 4] },
    { id: 'electronic', label: tr('Electronic'), bands: [6, 3, -2, 2.5, 5] },
    { id: 'hiphop', label: tr('Hip-Hop'), bands: [7, 3.5, 0, 1.5, 3] },
    { id: 'night', label: tr('Late Night'), bands: [3, 1, 0, -2, -4] },
  ]);
  function eqBandsMatch(a: EqBands, b: EqBands) {
    return a.every((g, i) => Math.abs(g - b[i]) < 0.05);
  }

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
  const XF_DEFAULTS = {
    crossfade_seconds: 4,
    album_continuity: true,
    crossfade_curve_out: 'equal_power',
    crossfade_curve_in: 'equal_power',
    crossfade_out_len: 100,
    crossfade_in_len: 100,
    crossfade_manual: true,
    crossfade_smart: false,
    crossfade_filter: 'off',
    bpm_match: false,
    bpm_max_change: 6,
    bpm_beat_align: true,
    bpm_return_seconds: 8,
    bpm_keylock: false,
  } satisfies Partial<SettingsT>;
  const XF_CURVES = $derived([
    { value: 'equal_power', label: tr('Equal power') },
    { value: 'linear', label: tr('Linear') },
    { value: 's_curve', label: tr('S-curve') },
    { value: 'logarithmic', label: tr('Logarithmic') },
    { value: 'exponential', label: tr('Exponential') },
  ]);
  let settings = $state<SettingsT>({ quality: 'high', language: 'en', region: 'US', notifications: false, discord_presence: true, autoplay: true, resume_on_start: false, cache_max_mb: 1024, offline_mode: false, crossfade: true, eq_enabled: false, eq_bands: [0, 0, 0, 0, 0], listening_stats: true, shortcuts: {}, global_hotkeys: {}, tray_icon: false, close_to_tray: false, normalize: false, normalize_target: -14, auto_dj: false, lyrics_karaoke: true, lyrics_glow: true, lyrics_blur: true, lyrics_dots: true, lyrics_size: 100, ...XF_DEFAULTS });
  const eqPresetId = $derived(
    EQ_PRESETS.find((p) => eqBandsMatch(settings.eq_bands, p.bands))?.id ?? null,
  );
  function resetCrossfade() {
    Object.assign(settings, XF_DEFAULTS);
    api.setSettings(settings);
  }
  let player = $state<PlayerState>({
    track: null, playing: false, loading: false, buffering: false, buffered: 0,
    position: 0, duration: 0,
    volume: 0.85, queue: [], index: 0, error: null, accent: null,
    shuffle: false, repeat: 'off', liked: false, disliked: false, autoplay: true,
  });
  let loading = $state(false);
  // Local "find in this list" filters. Cleared whenever the view changes.
  let filterQ = $state('');
  let sideQ = $state('');
  let fCollection = $derived(collection ? filterTracks(collection.tracks, filterQ) : []);
  const LIKED_PREVIEW = 10;
  let likedAll = $state(false);
  let fLiked = $derived(filterTracks((library?.liked ?? []).filter((t) => !off.offline || off.playable.has(t.id)), filterQ));
  let likedShown = $derived(likedAll ? fLiked : fLiked.slice(0, LIKED_PREVIEW));
  let fPlaylists = $derived(filterNamed((library?.playlists ?? []).filter((p) => !off.offline || offSrcIds.has(p.id)), filterQ));
  let fAlbums = $derived(filterNamed((library?.albums ?? []).filter((a) => !off.offline || offSrcIds.has(a.id)), filterQ, (a) => a.subtitle ?? ''));
  let fArtists = $derived(off.offline ? [] : filterNamed(library?.artists ?? [], filterQ));
  let fSide = $derived(filterNamed(library?.playlists ?? [], sideQ));
  let fHistory = $derived(filterTracks(history ?? [], filterQ));
  // ---- offline / downloads ----
  const ONLINE_ONLY = new Set(['home', 'explore', 'search', 'charts', 'new-releases', 'genre']);
  let dlTracks = $state<Track[]>([]);
  let fDl = $derived(filterTracks(dlTracks, filterQ));
  let confirmRm = $state<string | null>(null);
  let colSourceId = $derived(
    view.id === 'playlist' ? view.playlistId : view.id === 'album' ? view.albumId : null,
  );
  let colSrc = $derived(sourceInfo(colSourceId));
  let dlBusy = $derived((off.status?.queued ?? 0) + (off.status?.active.length ?? 0) > 0);
  let dlDone = $derived(
    (off.status?.sources ?? []).reduce((n, s) => n + s.done, 0),
  );
  let dlTotal = $derived(
    (off.status?.sources ?? []).reduce((n, s) => n + s.total, 0),
  );
  // Offline library: only what is on disk.
  let offSrcIds = $derived(new Set((off.status?.sources ?? []).map((s) => s.id)));
  let sidePlaylists = $derived(
    off.offline ? fSide.filter((p) => offSrcIds.has(p.id)) : fSide,
  );
  $effect(() => {
    void JSON.stringify(view);
    filterQ = '';
    confirmRm = null;
  });
  $effect(() => {
    const m = off.mode;
    if (settings.offline_mode !== m) settings.offline_mode = m;
  });
  // Keep the "Downloaded songs" list fresh while the Downloads page is open.
  $effect(() => {
    void off.status?.downloaded.length;
    if (view.id === 'downloads') loadDownloads();
  });
  // Seek-bar waveform of the current track (polled until it is analysed).
  let wave = $state<number[] | null>(null);
  // Derived so the poll restarts only when the track changes, not on every tick.
  let waveId = $derived(player.track?.id);
  $effect(() => {
    const id = waveId;
    const on = !!settings.seek_waveform;
    wave = null;
    if (!id || !on) return;
    let stop = false;
    let tries = 0;
    const poll = () => {
      if (stop) return;
      api.trackWave(id).then((w) => {
        if (stop) return;
        if (w && w.length) wave = w;
        else if (++tries < 60) timer = setTimeout(poll, tries < 5 ? 1500 : 4000);
      }).catch(() => {});
    };
    let timer: ReturnType<typeof setTimeout> | undefined = setTimeout(poll, 300);
    return () => { stop = true; clearTimeout(timer); };
  });
  // Playback speed / reverb presets (the backend clamps and applies them).
  const FX_PRESETS: Record<string, { speed: number; speed_keep_pitch: boolean; reverb: number }> = {
    normal: { speed: 1, speed_keep_pitch: false, reverb: 0 },
    nightcore: { speed: 1.25, speed_keep_pitch: false, reverb: 0 },
    slowed: { speed: 0.85, speed_keep_pitch: false, reverb: 0.35 },
  };
  let fxPreset = $derived.by(() => {
    const s = settings.speed ?? 1, k = !!settings.speed_keep_pitch, r = settings.reverb ?? 0;
    for (const [name, p] of Object.entries(FX_PRESETS)) {
      if (Math.abs(p.speed - s) < 0.001 && p.speed_keep_pitch === k && Math.abs(p.reverb - r) < 0.001) return name;
    }
    return 'custom';
  });
  function applyFxPreset(name: string) {
    const p = FX_PRESETS[name];
    if (p) Object.assign(settings, p);
    else if (fxPreset === 'normal') Object.assign(settings, { speed: 1.1, speed_keep_pitch: true, reverb: 0 });
    api.setSettings(settings);
  }
  function fmtSpeed(v: number): string {
    return `${(Math.round(v * 100) / 100).toString()}×`;
  }
  let libLoading = $state(false);
  // True while `library` holds only the cached / sidebar-fast subset.
  let libPartial = $state(false);
  let chip = $state<string | null>(null);
  let mainEl: HTMLElement | undefined = $state();
  let likes = $state<Track[]>(localLikes());
  // Disliked song ids, remembered here so the thumbs-down stays lit when the
  // song comes up again (the player state only knows the current track).
  const DISLIKE_KEY = 'tarn.disliked.v1';
  let dislikedIds = $state<string[]>((() => {
    try { const v = JSON.parse(localStorage.getItem(DISLIKE_KEY) ?? '[]'); return Array.isArray(v) ? v.slice(-2000) : []; } catch { return []; }
  })());
  function isDisliked(id: string) { return dislikedIds.includes(id); }
  function setDisliked(id: string, on: boolean) {
    const has = dislikedIds.includes(id);
    if (on === has) return;
    dislikedIds = on ? [...dislikedIds, id].slice(-2000) : dislikedIds.filter((x) => x !== id);
    try { localStorage.setItem(DISLIKE_KEY, JSON.stringify(dislikedIds)); } catch {}
  }
  let recents = $state<Track[]>(localRecents());
  type HistEntry = { view: View; scroll: number; searchQ: string; results: SearchResults | null; searchTab: typeof searchTab };
  let backStack = $state<HistEntry[]>([]);
  let fwdStack = $state<HistEntry[]>([]);
  let navLock = false;
  type PanelId = 'queue' | 'lyrics' | 'related';
  let queueOn = $state(false);
  let lyricsOn = $state(false);
  let relatedOn = $state(false);
  /** Which side-panel body to show when not fullscreen. */
  let panelFocus = $state<PanelId | null>(null);
  let sidePanelOn = $derived(queueOn || lyricsOn || relatedOn);
  let sidePanelView = $derived.by((): PanelId | null => {
    if (panelFocus === 'queue' && queueOn) return 'queue';
    if (panelFocus === 'lyrics' && lyricsOn) return 'lyrics';
    if (panelFocus === 'related' && relatedOn) return 'related';
    if (relatedOn) return 'related';
    if (lyricsOn) return 'lyrics';
    if (queueOn) return 'queue';
    return null;
  });
  let expanded = $state(false);
  let npShow = $state(true);
  let npHoverChrome = $state(false);
  let npHideTimer: ReturnType<typeof setTimeout> | undefined;
  let npSlide = $state<'next' | 'prev' | null>(null);
  let npPrevId = '';
  let npPrevIndex = 0;
  let scrub = $state<number | null>(null);
  let volHud = $state<number | null>(null);
  let volHudTimer: ReturnType<typeof setTimeout> | undefined;
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

  let accentWasFixed = false;
  $effect(() => {
    // A fixed accent from settings wins over the colour of the cover art.
    const fixed = hexRgb(settings.accent_color);
    const rgb = fixed ?? player.accent;
    const root = document.documentElement.style;
    if (!rgb && accentWasFixed) {
      // Fixed accent turned off with nothing playing: back to the default.
      for (const p of ['--accent-rgb', '--accent', '--accent-dim', '--accent-fg', '--accent-fg-rgb']) root.removeProperty(p);
    }
    accentWasFixed = !!fixed;
    if (rgb) {
      const [r, g, b] = rgb;
      const paint = paintAccent(r, g, b);
      root.setProperty('--accent-rgb', paint.rgb);
      root.setProperty('--accent', paint.css);
      root.setProperty('--accent-dim', paint.dim);
      root.setProperty('--accent-fg', paint.fg);
      root.setProperty('--accent-fg-rgb', paint.fgRgb);
    }
  });
  $effect(() => {
    // Undefined until the saved settings arrive (main.ts painted the last theme).
    if (settings.theme !== undefined) applyTheme(settings.theme);
  });

  onMount(() => {
    const unsubs: Array<() => void> = [];
    (async () => {
      /* home cache is account-keyed; filled after auth() below */
      likes = localLikes();
      recents = localRecents();
      try {
        player = await api.state();
        // Both the global hotkeys and the tray menu can ask to like the track.
        for (const ev of ['hotkey', 'tray']) {
          unsubs.push(await listen<string>(ev, (e) => {
            if (e.payload === 'like' && player.track) void toggleLike();
          }));
        }
        unsubs.push(await listen<string>('toast', (e) => {
          if (e.payload) toast(String(e.payload), 'info', 3600);
        }));
        await initOffline((offline, was) => {
          if (offline && !was) {
            toast(off.mode ? tr('Offline mode - only downloaded songs play') : tr("You're offline - downloaded music still plays"), 'info', 4200);
            if (ONLINE_ONLY.has(view.id)) go({ id: 'downloads' }, { replace: true });
          } else if (!offline && was) {
            toast(tr('Back online'), 'info', 2500);
            if (auth.signed_in) loadLibrary({ retries: 1 });
            if (view.id === 'home') loadHome();
          }
        });
        auth = await api.auth();
        settings = await api.settings();
        setLocale(settings.ui_language);
        if (!Array.isArray(settings.eq_bands) || settings.eq_bands.length !== 5) {
          settings.eq_bands = [0, 0, 0, 0, 0];
        }
        settings.eq_enabled = !!settings.eq_enabled;
        if (settings.listening_stats == null) settings.listening_stats = true;
        settings.shortcuts ??= {};
        settings.global_hotkeys ??= {};
        settings.normalize = !!settings.normalize;
        if (settings.normalize_target == null || Number.isNaN(Number(settings.normalize_target))) settings.normalize_target = -14;
        if (settings.crossfade_manual == null) settings.crossfade_manual = true;
        if (settings.cache_max_mb == null || Number.isNaN(Number(settings.cache_max_mb))) {
          settings.cache_max_mb = 1024;
        }
        syncCacheCustomFromSettings();
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
      if (auth.signed_in) {
        loadLibrary();
      } else {
        // restore_session may finish after first auth(); listen('auth') handles
        // the happy path, this is a backup if the event is missed.
        setTimeout(() => {
          api.auth().then((a) => {
            if (!a.signed_in) return;
            auth = a;
            loadLibrary({ retries: 4 });
            if (!home) loadHome();
          }).catch(() => {});
        }, 900);
      }
    })();
    import('@tauri-apps/api/app').then((m) => m.getVersion()).then((v) => { appVersion = v; }).catch(() => {});
    listen<PlayerState>('player', (e) => {
      const next = e.payload;
      // The backend leaves the queue out of unchanged updates (it can be
      // thousands of tracks): keep the same array so nothing is re-parsed,
      // re-proxied or re-rendered five times a second.
      if (next.queue_same) next.queue = player.queue;
      if (next.track) {
        const id = next.track.id;
        const known =
          isLocalLiked(id) ||
          !!library?.liked?.some((t) => t.id === id);
        if (known && !next.liked) {
          next.liked = true;
          next.disliked = false;
        }
      }
      if (next.track && !next.liked && isDisliked(next.track.id)) next.disliked = true;
      // A like/dislike still in flight: keep showing what the user just chose.
      if (pendingRating && next.track?.id === pendingRating.id) {
        next.liked = pendingRating.liked;
        next.disliked = pendingRating.disliked;
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
          loadLibrary({ retries: was ? 2 : 4 });
          loadHome();
          if (view.id === 'explore') loadExplore();
          for (const t of localLikes()) {
            api.like(t.id, 'like').catch(() => {});
          }
        } else if (was) {
          library = null;
          loadHome();
        }
      });
    }).then((u) => unsubs.push(u));

    const keys = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      const typing = tag === 'INPUT' || tag === 'TEXTAREA' || (e.target as HTMLElement)?.isContentEditable;
      const combo = appCombo(e);
      const action = combo ? actionFor(appShortcuts(settings.shortcuts), combo) : null;
      if (action === 'search' && !typing) {
        e.preventDefault();
        go({ id: 'search' });
        queueMicrotask(() => document.getElementById('q')?.focus());
        return;
      }
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'f') {
        const el = document.getElementById('filter-input');
        if (el) {
          e.preventDefault();
          el.focus();
          (el as HTMLInputElement).select();
          return;
        }
      }
      if (!typing && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
        e.preventDefault();
        return;
      }
      if (typing) return;
      if ((e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) || e.key === 'BrowserBack' || e.key === 'BrowserForward') {
        e.preventDefault();
        if (e.key === 'ArrowLeft' || e.key === 'BrowserBack') goBack();
        else goFwd();
        return;
      }
      if (action === 'play_pause' || e.code === 'MediaPlayPause') {
        e.preventDefault();
        api.toggle();
      } else if (action === 'next') {
        e.preventDefault();
        api.next();
      } else if (action === 'prev') {
        e.preventDefault();
        api.prev();
      } else if (action === 'seek_forward') {
        e.preventDefault();
        api.seek(player.position + 5);
      } else if (action === 'seek_back') {
        e.preventDefault();
        api.seek(Math.max(0, player.position - 5));
      } else if (action === 'volume_up' || action === 'volume_down') {
        e.preventDefault();
        setVolumePrecise(player.volume + (action === 'volume_up' ? 0.05 : -0.05));
      } else if (action === 'like') {
        e.preventDefault();
        void toggleLike();
      } else if (e.code === 'MediaTrackNext') {
        api.next();
      } else if (e.code === 'MediaTrackPrevious') {
        api.prev();
      } else if (e.key === 'F11') {
        e.preventDefault();
        void toggleWindowFullscreen();
      } else if (e.key === 'Escape') {
        if (expanded) expanded = false;
        else if (sidePanelOn) closePanels();
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
    if (v.id === 'downloads') loadDownloads();
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
    if (off.offline) {
      if (ONLINE_ONLY.has(v.id)) v = { id: 'downloads' };
      else if (v.id === 'artist') {
        toast(tr('Artist pages need a connection'), 'info', 2600);
        return;
      }
    }
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

  // iOS webviews often refuse clipboard writes; then show the text so it can be selected by hand.
  let diagText = $state<string | null>(null);

  let setQuery = $state('');
  let setEl = $state<HTMLElement | null>(null);
  let setEmpty = $state(false);
  function applySetFilter() {
    if (!setEl) return;
    const q = setQuery.trim().toLowerCase();
    let any = false;
    setEl.querySelectorAll<HTMLElement>('.set-sec').forEach((sec) => {
      const titleHit = !!q && (sec.querySelector('.set-h')?.textContent ?? '').toLowerCase().includes(q);
      let shown = 0;
      sec.querySelectorAll<HTMLElement>('.set-card > *').forEach((row) => {
        const hit = !q || titleHit || (row.textContent ?? '').toLowerCase().includes(q);
        row.style.display = hit ? '' : 'none';
        if (hit) shown++;
      });
      sec.style.display = shown ? '' : 'none';
      if (shown) any = true;
    });
    const nav = setEl.querySelector<HTMLElement>('.set-nav');
    if (nav) nav.style.display = q ? 'none' : '';
    setEmpty = !!q && !any;
  }
  $effect(() => {
    void setQuery;
    if (!setEl) return;
    applySetFilter();
    const mo = new MutationObserver(() => applySetFilter());
    mo.observe(setEl, { childList: true, subtree: true });
    return () => mo.disconnect();
  });
  async function copyDiagnostics() {
    let text = '';
    try {
      text = await api.diagnostics();
    } catch (e) {
      fail(e, "Couldn't read diagnostics");
      return;
    }
    try {
      await navigator.clipboard.writeText(text);
      toast(tr('Diagnostics copied'), 'info', 2500);
    } catch {
      diagText = text;
    }
  }
  let homeInflight: Promise<void> | null = null;
  /** Startup triggers Home from several places; share one request. */
  function loadHome(params?: string | null): Promise<void> {
    if (params) return loadHomeNow(params);
    homeInflight ??= loadHomeNow(params).finally(() => { homeInflight = null; });
    return homeInflight;
  }
  async function loadHomeNow(params?: string | null) {
    if (off.offline) {
      loading = false;
      if (view.id === 'home') go({ id: 'downloads' }, { replace: true });
      return;
    }
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
    if (off.offline) return;
    loading = true;
    try { explore = await api.explore(); } catch (e) { fail(e, "Couldn't load Explore"); } finally { loading = false; }
  }
  async function loadArtist(id: string) {
    loading = true; artist = null;
    try { artist = await api.artist(id); } catch (e) { fail(e, "Couldn't load this artist"); } finally { loading = false; }
  }
  /** Playlist/album with a downloaded copy as the offline (or failure) fallback. */
  async function openCollection(kind: 'album' | 'playlist', id: string): Promise<Collection> {
    const local = () => api.offlineCollection(id).catch(() => null);
    if (off.offline) {
      const c = await local();
      if (c) return c;
      throw new Error("Not downloaded - available when you're back online");
    }
    try {
      return kind === 'album' ? await api.album(id) : await api.playlist(id);
    } catch (e) {
      const c = await local();
      if (c) return c;
      throw e;
    }
  }
  async function toggleDownload() {
    const c = collection;
    const id = colSourceId;
    if (!c || !id) return;
    const have = sourceInfo(id);
    try {
      if (have) {
        if (have.done < have.total) {
          await api.offlineCancel(id);
          toast(tr('Download cancelled'), 'info', 2200);
        } else if (confirmRm === id) {
          await api.offlineRemove(id);
          confirmRm = null;
          toast(tr('Download removed'), 'info', 2200);
        } else {
          // Second tap on a finished download removes it.
          confirmRm = id;
          toast(tr('Tap again to remove this download'), 'info', 2400);
          setTimeout(() => { if (confirmRm === id) confirmRm = null; }, 3000);
        }
      } else {
        const tracks = c.tracks.filter((t) => t.id);
        if (!tracks.length) return;
        await api.offlineDownload(
          { id, kind: c.kind === 'album' ? 'album' : 'playlist', name: c.name, cover: c.cover ?? null, subtitle: c.author || artistsLine(c.artists) || null },
          tracks,
        );
        toast(tr('Downloading {n} songs', { n: tracks.length }), 'info', 2600);
      }
      refreshOffline();
    } catch (e) { fail(e, "Couldn't change this download"); }
  }
  async function toggleLikedDownload() {
    const have = sourceInfo('liked');
    try {
      if (have && have.done < have.total) {
        await api.offlineCancel('liked');
        toast(tr('Download cancelled'), 'info', 2200);
      } else if (have) {
        if (confirmRm === 'liked') {
          await api.offlineRemove('liked');
          confirmRm = null;
          toast(tr('Download removed'), 'info', 2200);
        } else {
          confirmRm = 'liked';
          toast(tr('Tap again to remove downloaded liked songs'), 'info', 2400);
          setTimeout(() => { if (confirmRm === 'liked') confirmRm = null; }, 3000);
        }
      } else {
        const tracks = (library?.liked ?? []).filter((t) => t.id);
        if (!tracks.length) return;
        await api.offlineDownload({ id: 'liked', kind: 'liked', name: 'Liked songs', cover: null, subtitle: null }, tracks);
        toast(tr('Downloading {n} liked songs', { n: tracks.length }), 'info', 2600);
      }
      refreshOffline();
    } catch (e) { fail(e, "Couldn't change this download"); }
  }
  async function loadDownloads() {
    try { dlTracks = await api.offlineTracks(); } catch { dlTracks = []; }
  }
  async function loadAlbum(id: string) {
    playlistSeq++; // an unfinished playlist load must not overwrite this album
    loading = true; collection = null;
    try { collection = await openCollection('album', id); } catch (e) { fail(e, "Couldn't load this album"); } finally { loading = false; }
  }
  // A newer open (or leaving the page) makes the slower, older load drop its result.
  let playlistSeq = 0;
  const HEAD_TRACKS = 100;
  // Playlists already opened: painted at once next time (like Liked), then
  // refreshed. Whole lists live in memory; the first batch also survives a
  // restart via localStorage.
  const PL_CACHE_KEY = 'tarn.plcache.v1';
  const plMem = new Map<string, Collection>();
  function plDisk(): Record<string, Collection> {
    try { return JSON.parse(localStorage.getItem(PL_CACHE_KEY) || '{}'); } catch { return {}; }
  }
  function plCached(id: string): Collection | null {
    const bare = id.replace(/^VL/, '');
    return plMem.get(bare) ?? plDisk()[bare] ?? null;
  }
  function plRemember(id: string, c: Collection) {
    const bare = id.replace(/^VL/, '');
    plMem.set(bare, c);
    try {
      const all = plDisk();
      delete all[bare]; // re-insert last so the newest stay when trimming
      all[bare] = { ...c, tracks: c.tracks.slice(0, HEAD_TRACKS) };
      const keys = Object.keys(all);
      for (const k of keys.slice(0, Math.max(0, keys.length - 25))) delete all[k];
      localStorage.setItem(PL_CACHE_KEY, JSON.stringify(all));
    } catch {}
  }
  async function loadPlaylist(id: string) {
    const seq = ++playlistSeq;
    const stale = () => seq !== playlistSeq;
    loading = true; collection = null;
    try {
      const isLiked = id === 'LM' || id === 'VLLM';
      if (isLiked && library?.liked?.length) {
        // Paint the first batch from the cached library at once; the whole
        // list replaces it when the load below finishes.
        const all = library.liked;
        collection = {
          kind: 'playlist', id: 'LM', playlist_id: 'LM', name: 'Liked Music',
          cover: all[0]?.cover ?? null, artists: [], artist_ids: [], author: null,
          year: null, description: null, track_count: all.length, album_type: null,
          tracks: all.slice(0, HEAD_TRACKS), related: [], owned: true, saved: true,
        };
        loading = false;
      } else if (!isLiked && plCached(id)) {
        // Seen before: show it instantly, refresh below.
        collection = plCached(id);
        loading = false;
      } else if (!isLiked && !off.offline) {
        // First page now, the rest right after, so a big playlist opens instantly.
        const head = await api.playlistHead(id);
        if (stale()) return;
        collection = head;
        loading = false;
      }
      const full = await openCollection('playlist', id);
      if (stale()) return;
      collection = full;
      if (!isLiked) plRemember(id, full);
    } catch (e) {
      if (stale()) return;
      // Keep whatever is already on screen if only the full load failed.
      if (!collection) fail(e, "Couldn't load this playlist");
    } finally {
      if (!stale()) loading = false;
    }
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
  async function loadLibrary(opts?: { retries?: number }) {
    if (!auth.signed_in) {
      libLoading = false;
      return;
    }
    if (!library) {
      libLoading = true;
      // Paint something immediately: last session's library from disk, else
      // the sidebar-fast subset, while the full (1500+ tracks) load runs.
      try {
        const cached = await api.libraryCached();
        if (cached && !library) { library = cached; libPartial = true; }
      } catch {}
      if (!library) {
        try {
          const lite = await api.libraryLite();
          if (!library && lite.signed_in && (lite.playlists?.length || lite.artists?.length || lite.albums?.length)) {
            library = lite;
            libPartial = true;
          }
        } catch {}
      }
    }
    if (off.offline) {
      // No network: whatever was cached on disk is the library.
      if (!library) {
        try { library = await api.libraryCached(); } catch {}
      }
      libLoading = false;
      libPartial = false;
      return;
    }
    const retries = opts?.retries ?? 3;
    try {
      let last = await api.library();
      // Session restore / cookie settle can return an empty library once;
      // retry a few times so the sidebar playlists show without a manual reload.
      for (let i = 0; i < retries; i++) {
        const empty =
          !last.playlists?.length &&
          !last.liked?.length &&
          !last.albums?.length &&
          !last.artists?.length;
        if (!empty) break;
        await new Promise((r) => setTimeout(r, 350 + i * 250));
        if (!auth.signed_in) break;
        last = await api.library();
      }
      library = last;
    } catch (e) {
      fail(e, "Couldn't load your library");
      if (!library) {
        library = { signed_in: auth.signed_in, mock: auth.mock, account: auth.account, playlists: [], liked: [], albums: [], artists: [], recents: [] };
      }
    } finally {
      libLoading = false;
      libPartial = false;
    }
  }
  async function loadHistory() {
    loading = true;
    if (off.offline) { history = []; loading = false; return; }
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

  function withCoverFallback(tracks: Track[], fallback?: string | null): Track[] {
    if (!fallback) return tracks;
    return tracks.map((t) => (t.cover ? t : { ...t, cover: fallback }));
  }

  /** Player bar / NP art - album/queue art, then public YouTube thumb. */
  let nowCover = $derived.by(() => {
    const t = player.track;
    if (!t) return null;
    if (t.cover) return coverFor(t);
    const fromQueue = player.queue.find((x) => x.id === t.id && x.cover);
    if (fromQueue) return coverFor(fromQueue);
    if (collection?.cover && collection.tracks.some((x) => x.id === t.id)) {
      return coverFor({ id: t.id, cover: collection.cover });
    }
    return coverFor(t);
  });

  // Skipping with the fullscreen player (lyrics / up next) remounts the art;
  // load the neighbours' covers ahead so it paints at once instead of
  // flickering while it streams in.
  $effect(() => {
    const q = player.queue;
    const i = player.index;
    if (!q?.length) return;
    for (const t of [q[i], q[i + 1], q[i + 2], q[i - 1]]) {
      if (t) preloadNowCover(coverFor(t));
    }
  });

  async function playTracks(tracks: Track[], index: number, fallbackCover?: string | null) {
    if (!tracks.length) return;
    const list = withCoverFallback(tracks, fallbackCover ?? null);
    const t = list[index] ?? list[0];
    recents = recordRecent(t);
    try {
      await api.play(t, list, index);
    } catch (e) {
      fail(e, "Can't play this track");
    }
  }
  // Replace the queue - never keep leftover tracks from the previous playlist.
  function playOne(t: Track) { playTracks([t], 0); }

  async function startRadio(id?: string) {
    const vid = id || player.track?.id;
    if (!vid) return;
    try { await api.radio(vid); } catch (e) { fail(e, "Couldn't start radio"); }
  }

  async function playTopShuffled(top: ShelfItem) {
    const songs = results?.songs?.length ? results.songs : [itemToTrack(top)];
    // Backend reshuffles the live queue so Up next matches; keep original order here.
    await playTracks(songs, 0);
    api.setShuffle(true);
  }

  async function playCollection(item: ShelfItem, shuffle: boolean) {
    try {
      let tracks: Track[] = [];
      let cover: string | null = item.cover ?? null;
      if (item.kind === 'artist') {
        const a = await api.artist(item.id || item.browse_id || '');
        tracks = a.tracks;
        cover = a.cover ?? cover;
      } else if (item.kind === 'album') {
        const c = await openCollection('album', item.id || item.browse_id || '');
        tracks = c.tracks;
        cover = c.cover ?? cover;
      } else if (item.kind === 'playlist') {
        const c = await openCollection('playlist', item.id);
        tracks = c.tracks;
        cover = c.cover ?? cover;
      } else if (item.kind === 'track' || item.kind === 'video') {
        tracks = [itemToTrack(item)];
      }
      if (!tracks.length) { toast(tr('This playlist is empty'), 'info', 2600); openItem(item); return; }
      await playTracks(tracks, 0, cover);
      if (shuffle) api.setShuffle(true);
      else api.setShuffle(false);
    } catch (e) { fail(e, "Can't play this"); }
  }

  function playAlbumId(id: string, cover?: string | null) {
    playCollection({
      kind: 'album', id, title: '', cover: cover ?? null, subtitle: null,
      artists: [], artist_ids: [], album: null, album_id: null, duration: null,
      playlist_id: null, color: null, browse_id: id, params: null, year: null,
      album_type: null, track_count: null, is_video: false,
    }, false);
  }
  function playPlaylistId(id: string, cover?: string | null, covers?: string[]) {
    playCollection({
      kind: 'playlist', id, title: '', cover: cover ?? covers?.[0] ?? null, subtitle: null,
      artists: [], artist_ids: [], album: null, album_id: null, duration: null,
      playlist_id: id, color: null, browse_id: null, params: null, year: null,
      album_type: null, track_count: null, is_video: false,
    }, false);
  }
  function playArtistId(id: string, cover?: string | null) {
    playCollection({
      kind: 'artist', id, title: '', cover: cover ?? null, subtitle: null,
      artists: [], artist_ids: [], album: null, album_id: null, duration: null,
      playlist_id: null, color: null, browse_id: id, params: null, year: null,
      album_type: null, track_count: null, is_video: false,
    }, false);
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
    if (item.kind === 'playlist' || item.kind === 'album' || item.kind === 'artist') {
      ctxColl(e, {
        kind: item.kind, id: item.kind === 'artist' ? (item.id || item.browse_id || '') : item.id,
        title: item.title, cover: item.cover, subtitle: item.subtitle,
      });
      return;
    }
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

  // Reflect a like/unlike in the lists on screen right away (Library > Liked
  // and the Liked Music playlist page) instead of waiting for a reload.
  function syncLikedViews(track: Track, liked: boolean) {
    if (library) {
      const rest = (library.liked ?? []).filter((t) => t.id !== track.id);
      library = { ...library, liked: liked ? [track, ...rest] : rest };
    }
    const c = collection;
    if (c && (c.id === 'LM' || c.playlist_id === 'LM' || c.id === 'VLLM')) {
      const rest = c.tracks.filter((t) => t.id !== track.id);
      const tracks = liked ? [track, ...rest] : rest;
      collection = { ...c, tracks, track_count: tracks.length };
    }
  }

  // Rating the user just chose that the server hasn't confirmed yet.
  let pendingRating: { id: string; liked: boolean; disliked: boolean } | null = null;
  const likeSeq = new Map<string, number>();

  let homeRefreshTimer: ReturnType<typeof setTimeout> | undefined;
  /** YouTube learns from the dislike: hide the song now, re-pull the feed shortly. */
  function refreshHomeAfterDislike(id: string) {
    if (home) {
      home = {
        ...home,
        shelves: home.shelves.map((s) => ({
          ...s,
          items: s.items.filter((it) => !(it.kind === 'track' && it.id === id)),
        })),
      };
    }
    clearHomeCaches();
    clearTimeout(homeRefreshTimer);
    homeRefreshTimer = setTimeout(() => { if (!off.offline && auth.signed_in) void loadHome(); }, 2500);
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
    const id = track.id;
    const want =
      action === 'like' ? { liked: true, disliked: false }
      : action === 'dislike' ? { liked: false, disliked: true }
      : { liked: false, disliked: false };
    const current = player.track?.id === id;
    const was = {
      liked: current ? player.liked : isLocalLiked(id) || !!library?.liked?.some((t) => t.id === id),
      disliked: current ? player.disliked : isDisliked(id),
    };
    setDisliked(id, want.disliked);
    if (action === 'dislike') toast(tr('Disliked'), 'info', 2200);
    const seq = (likeSeq.get(id) ?? 0) + 1;
    likeSeq.set(id, seq);

    // Instant: heart, local likes, Library > Liked and the Liked Music page.
    pendingRating = { id, ...want };
    if (current) { player.liked = want.liked; player.disliked = want.disliked; }
    likes = setLocalLiked(track, want.liked);
    syncLikedViews(track, want.liked);

    // Disliked: stop hearing it - skip if it's playing, drop it from Up next.
    if (action === 'dislike') {
      const ahead: number[] = [];
      player.queue.forEach((t, i) => { if (t.id === id && i > player.index) ahead.push(i); });
      for (const i of ahead.reverse()) api.queueRemove(i).catch(() => {});
      if (current) api.next().catch(() => {});
    }

    try {
      const st = await api.like(id, action, track);
      if (likeSeq.get(id) !== seq) return; // a newer tap on this song wins
      if (player.track?.id === id) { player.liked = st.liked; player.disliked = st.disliked; }
      if (action === 'dislike') refreshHomeAfterDislike(id);
      if (st.liked !== want.liked) {
        likes = setLocalLiked(track, st.liked);
        syncLikedViews(track, st.liked);
      }
    } catch (e) {
      if (likeSeq.get(id) === seq) {
        // Roll it back, then say why.
        if (player.track?.id === id) { player.liked = was.liked; player.disliked = was.disliked; }
        setDisliked(id, was.disliked);
        likes = setLocalLiked(track, was.liked);
        syncLikedViews(track, was.liked);
      }
      fail(e, "Couldn't update this", action === 'dislike' ? 'dislike' : 'like');
    } finally {
      if (likeSeq.get(id) === seq && pendingRating?.id === id) pendingRating = null;
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

  function closePanels() {
    queueOn = false;
    lyricsOn = false;
    relatedOn = false;
    panelFocus = null;
  }

  async function openPanel(tab: PanelId) {
    if (tab === 'lyrics' && lyricsReady && !lyricsOk && !lyricsOn) return;
    if (expanded) {
      // Fullscreen: queue + lyrics can both stay open.
      if (tab === 'queue') {
        queueOn = !queueOn;
        if (queueOn) panelFocus = 'queue';
        return;
      }
      if (tab === 'lyrics') {
        lyricsOn = !lyricsOn;
        if (lyricsOn) panelFocus = 'lyrics';
        return;
      }
      relatedOn = !relatedOn;
      if (relatedOn) {
        queueOn = false;
        lyricsOn = false;
        panelFocus = 'related';
      } else {
        panelFocus = queueOn ? 'queue' : lyricsOn ? 'lyrics' : null;
      }
      return;
    }
    // Side panel: exclusive tabs.
    if (tab === 'queue') {
      const on = !queueOn;
      queueOn = on;
      lyricsOn = false;
      relatedOn = false;
      panelFocus = on ? 'queue' : null;
    } else if (tab === 'lyrics') {
      const on = !lyricsOn;
      lyricsOn = on;
      queueOn = false;
      relatedOn = false;
      panelFocus = on ? 'lyrics' : null;
    } else {
      const on = !relatedOn;
      relatedOn = on;
      queueOn = false;
      lyricsOn = false;
      panelFocus = on ? 'related' : null;
    }
  }

  // Fetch lyrics/related once per track. A plain (non-reactive) last-id
  // is required: the player snapshot is replaced every 200 ms and a
  // naive $effect would refetch in a loop.
  let relatedFor = '';
  let lyricsFor = '';
  let lyricsOk = $state(false);
  let lyricsReady = $state(false);
  $effect(() => {
    const id = player.track?.id ?? '';
    if (!id) {
      // Keep lyrics/queue panel mode across brief track gaps (next/crossfade).
      lyrics = null;
      lyricsFor = '';
      lyricsOk = false;
      lyricsReady = false;
      return;
    }
    if (lyricsFor !== id) {
      lyricsFor = id;
      lyrics = null;
      lyricsOk = false;
      lyricsReady = false;
      const req = id;
      (off.offline ? Promise.reject(new Error('offline')) : api.lyrics(id)).then((l) => {
        if (lyricsFor !== req) return;
        lyrics = l;
        lyricsOk = !!(l?.lines?.length);
        lyricsReady = true;
        // Stay on Lyrics mode even if this track has none / isn't synced.
      }).catch(() => {
        if (lyricsFor !== req) return;
        lyrics = { source: null, lines: [], synced: false };
        lyricsOk = false;
        lyricsReady = true;
      });
    }
    if (relatedOn && relatedFor !== id) {
      relatedFor = id;
      const req = id;
      (off.offline ? Promise.reject(new Error('offline')) : api.related(id)).then((r) => {
        if (relatedFor !== req) return;
        related = { ...r, tracks: dedupeTracks(r.tracks), versions: dedupeTracks(r.versions || []) };
      }).catch(() => {
        if (relatedFor === req) related = { tracks: [], versions: [], albums: [], artists: [], playlists: [] };
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

  function showVolHud(v: number) {
    volHud = Math.round(Math.max(0, Math.min(1, v)) * 100);
    clearTimeout(volHudTimer);
    volHudTimer = setTimeout(() => { volHud = null; }, 900);
  }

  function toggleShuffle() {
    const on = !player.shuffle;
    player.shuffle = on;
    api.setShuffle(on).catch((e) => { player.shuffle = !on; fail(e, "Couldn't change shuffle"); });
  }
  function cycleRepeatUi() {
    const prev = player.repeat;
    player.repeat = prev === 'off' ? 'all' : prev === 'all' ? 'one' : 'off';
    api.cycleRepeat()
      .then((r) => { player.repeat = r; })
      .catch((e) => { player.repeat = prev; fail(e, "Couldn't change repeat"); });
  }

  async function saveShortcuts(sc: Record<string, string>, gh: Record<string, string>) {
    const globalChanged = JSON.stringify(gh) !== JSON.stringify(settings.global_hotkeys);
    settings.shortcuts = sc;
    settings.global_hotkeys = gh;
    await api.setSettings(settings);
    if (!globalChanged) return;
    // Registration happens on the hotkey thread; give it a moment.
    await new Promise((r) => setTimeout(r, 400));
    try {
      const errs = await api.hotkeyErrors();
      if (errs.length) toast(tr("Couldn't register {keys} - try another combo", { keys: errs.map((e) => e.split(':')[0].replace(/_/g, ' ')).join(', ') }), 'error', 4200);
    } catch {}
  }

  function setVolumePrecise(v: number) {
    const next = Math.round(Math.max(0, Math.min(1, v)) * 100) / 100;
    if (Math.abs(next - player.volume) < 1e-6) return;
    player.volume = next;
    api.setVolume(next).catch(() => {});
    showVolHud(next);
  }

  function nudgeVolume(e: WheelEvent) {
    if (!player.track) return;
    e.preventDefault();
    const step = e.shiftKey ? 0.05 : 0.01;
    setVolumePrecise(player.volume + (e.deltaY > 0 ? -step : step));
  }

  function setEqBand(i: number, gain: number) {
    const bands = [...settings.eq_bands] as EqBands;
    bands[i] = Math.round(Math.max(-12, Math.min(12, gain)) * 10) / 10;
    settings.eq_bands = bands;
    if (!settings.eq_enabled) settings.eq_enabled = true;
    api.setSettings(settings);
  }

  function applyEqPreset(bands: EqBands) {
    settings.eq_bands = [...bands] as EqBands;
    settings.eq_enabled = true;
    api.setSettings(settings);
  }

  async function copyLink(opts: { video?: string; album?: string; playlist?: string; artist?: string }) {
    try {
      await navigator.clipboard.writeText(shareUrl(opts));
      toast(tr('Link copied'), 'info', 1800);
    } catch { toast(tr("Couldn't copy the link"), 'info', 2400); }
  }

  /** Right-click on a playlist / album / artist card. */
  function ctxColl(e: MouseEvent, c: CollTarget) {
    e.preventDefault();
    e.stopPropagation();
    ctx = {
      x: e.clientX, y: e.clientY, coll: c,
      playlistId: c.kind === 'playlist' ? c.id : undefined,
      albumId: c.kind === 'album' ? c.id : undefined,
      artistId: c.kind === 'artist' ? c.id : undefined,
    };
  }

  function mkItem(c: CollTarget): ShelfItem {
    return {
      kind: c.kind, id: c.id, title: c.title, cover: c.cover, subtitle: c.subtitle ?? null,
      artists: [], artist_ids: [], album: null, album_id: null, duration: null,
      playlist_id: c.kind === 'playlist' ? c.id : null, color: null,
      browse_id: c.kind === 'artist' ? c.id : null, params: null, year: null,
      album_type: null, track_count: null, is_video: false,
    };
  }

  async function collTracks(c: CollTarget): Promise<Track[]> {
    if (c.kind === 'artist') return (await api.artist(c.id)).tracks;
    return (await openCollection(c.kind, c.id)).tracks;
  }

  async function queueColl(c: CollTarget, next: boolean) {
    try {
      const tracks = (await collTracks(c)).filter((t) => t.id);
      if (!tracks.length) { toast(tr('Nothing to add'), 'info', 2200); return; }
      await api.queueAdd(tracks, next);
      toast(next ? tr('Playing {n} songs next', { n: tracks.length }) : tr('Added {n} songs to the queue', { n: tracks.length }), 'info', 2400);
    } catch (e) { fail(e, "Couldn't add this to the queue"); }
  }

  async function radioColl(c: CollTarget) {
    try {
      const first = (await collTracks(c)).find((t) => t.id);
      if (!first) { toast(tr('Nothing to start a radio from'), 'info', 2200); return; }
      await startRadio(first.id);
    } catch (e) { fail(e, "Couldn't start the radio"); }
  }

  async function downloadColl(c: CollTarget) {
    if (c.kind === 'artist') return;
    const have = sourceInfo(c.id);
    try {
      if (have) {
        await (have.done < have.total ? api.offlineCancel(c.id) : api.offlineRemove(c.id));
        toast(have.done < have.total ? tr('Download cancelled') : tr('Download removed'), 'info', 2200);
      } else {
        const col = await openCollection(c.kind, c.id);
        const tracks = col.tracks.filter((t) => t.id);
        if (!tracks.length) { toast(tr('Nothing to download'), 'info', 2200); return; }
        await api.offlineDownload(
          { id: c.id, kind: c.kind, name: col.name || c.title, cover: col.cover ?? c.cover, subtitle: col.author || artistsLine(col.artists) || null },
          tracks,
        );
        toast(tr('Downloading {n} songs', { n: tracks.length }), 'info', 2600);
      }
      refreshOffline();
    } catch (e) { fail(e, "Couldn't change this download"); }
  }

  function inLibrary(c: CollTarget): boolean {
    const id = c.id.replace(/^VL/, '');
    if (c.kind === 'playlist') return !!library?.playlists?.some((p) => p.id.replace(/^VL/, '') === id);
    if (c.kind === 'album') return !!library?.albums?.some((a) => a.id === c.id);
    return !!library?.artists?.some((a) => a.id === c.id);
  }

  function buildMenu(c: CtxTarget): MenuEntry[] {
    const out: MenuEntry[] = [];
    const sep = () => { if (out.length && !('sep' in out[out.length - 1])) out.push({ sep: true }); };
    const online = !off.offline;
    const t = c.track;
    if (t) {
      out.push({ label: tr('Play'), icon: PlayIcon, onclick: () => playOne(t) });
      out.push({ label: tr('Play next'), icon: PlayListAddIcon, onclick: () => { api.queueAdd([t], true); toast(tr('Playing next'), 'info', 1800); } });
      out.push({ label: tr('Add to queue'), icon: Queue02Icon, onclick: () => { api.queueAdd([t], false); toast(tr('Added to queue'), 'info', 1800); } });
      if (online) out.push({ label: tr('Start radio'), icon: Radio02Icon, onclick: () => startRadio(t.id) });
      if (jam.info.mode === 'joined') {
        out.push({ label: "Add to host's queue", icon: Queue02Icon, onclick: () => {
          api.jamAdd(t).then(() => toast("Added to the host's queue", 'info', 1800)).catch((e) => toast(String(e), 'error', 2600));
        } });
      }
      if (online) {
        sep();
        const likedNow = player.track?.id === t.id
          ? player.liked
          : isLocalLiked(t.id) || !!library?.liked?.some((x) => x.id === t.id);
        if (likedNow) out.push({ label: tr('Liked'), icon: FavouriteFilledIcon, onclick: () => likeTrack(t, 'removelike') });
        else out.push({ label: tr('Like'), icon: FavouriteIcon, onclick: () => likeTrack(t, 'like') });
        if (isDisliked(t.id)) out.push({ label: tr('Remove dislike'), icon: ThumbsDownIcon, onclick: () => likeTrack(t, 'removelike') });
        else out.push({ label: tr('Dislike'), icon: ThumbsDownIcon, onclick: () => likeTrack(t, 'dislike') });
        if (auth.signed_in) {
          out.push({
            label: tr('Add to playlist'), icon: Add01Icon,
            children: [
              ...(library?.playlists ?? []).filter((p) => p.id !== 'LM' && p.id !== 'SE').map((p): MenuEntry => ({ label: p.name, onclick: () => addToPlaylist(p.id, t.id) })),
              { sep: true },
              { label: tr('New playlist…'), onclick: () => addToPlaylist('', t.id) },
            ],
          });
        }
      }
      sep();
      const artistId = c.artistId || t.artist_ids?.[0];
      if (online && (artistId || t.artists?.[0])) out.push({ label: tr('Go to artist'), icon: UserIcon, onclick: () => goArtist(artistId || '', t.artists?.[0]) });
      const albumId = c.albumId || t.album_id;
      if (albumId) out.push({ label: tr('Go to album'), icon: Album02Icon, onclick: () => go({ id: 'album', albumId }) });
      if (online) out.push({ label: tr('Copy link'), icon: Link01Icon, onclick: () => copyLink({ video: t.id }) });
      return out;
    }
    const k = c.coll;
    if (!k) return out;
    out.push({ label: tr('Play'), icon: PlayIcon, onclick: () => playCollection(mkItem(k), false) });
    out.push({ label: tr('Shuffle'), icon: ShuffleIcon, onclick: () => playCollection(mkItem(k), true) });
    out.push({ label: tr('Play next'), icon: PlayListAddIcon, onclick: () => queueColl(k, true) });
    out.push({ label: tr('Add to queue'), icon: Queue02Icon, onclick: () => queueColl(k, false) });
    if (online) out.push({ label: tr('Start radio'), icon: Radio02Icon, onclick: () => radioColl(k) });
    sep();
    if (k.kind !== 'artist' && (online || sourceInfo(k.id))) {
      const have = sourceInfo(k.id);
      out.push({
        label: !have ? tr('Download') : have.done < have.total ? tr('Cancel download') : tr('Remove download'),
        icon: !have ? Download04Icon : have.done < have.total ? Cancel01Icon : Delete02Icon,
        onclick: () => downloadColl(k),
      });
    }
    if (online && auth.signed_in) {
      const saved = inLibrary(k);
      if (saved) out.push({ label: k.kind === 'artist' ? tr('Already subscribed') : tr('Already saved'), icon: CheckmarkCircle02Icon, disabled: true });
      out.push({
        label: saved ? tr('Remove from library') : k.kind === 'artist' ? tr('Subscribe') : tr('Save to library'),
        icon: Bookmark02Icon,
        onclick: async () => {
          toast(saved ? tr('Removed from library') : k.kind === 'artist' ? tr('Subscribed') : tr('Saved to library'), 'info', 2000);
          try {
            if (k.kind === 'artist') await api.subscribe(k.id, !saved);
            else await api.librarySave(k.id, !saved);
            loadLibrary({ retries: 0 });
          } catch (e) { fail(e, "Couldn't update your library", 'save'); }
        },
      });
    }
    sep();
    out.push({ label: tr('Open'), icon: ArrowRight01Icon, onclick: () => go(k.kind === 'artist' ? { id: 'artist', artistId: k.id } : k.kind === 'album' ? { id: 'album', albumId: k.id } : { id: 'playlist', playlistId: k.id }) });
    if (online) out.push({ label: tr('Copy link'), icon: Link01Icon, onclick: () => copyLink({ [k.kind]: k.id }) });
    return out;
  }

  let importOpen = $state(false);

  // Small always-on-top player window (same page, ?mini=1).
  async function openMini(at?: { x: number; y: number }) {
    try {
      const existing = await WebviewWindow.getByLabel('mini');
      if (existing) {
        // Opened from the tray: a second click closes the popup.
        if (at) await existing.close();
        else await existing.setFocus();
        return;
      }
      // Tray popup: appear next to the cursor (physical px -> css px).
      let pos: { x?: number; y?: number } = {};
      if (at && at.x >= 0) {
        const d = window.devicePixelRatio || 1;
        const cx = at.x / d, cy = at.y / d;
        const x = Math.max(8, Math.min(window.screen.width - 360 - 8, cx - 180));
        const y = cy > window.screen.height / 2 ? cy - 110 - 14 : cy + 14;
        pos = { x: Math.round(x), y: Math.round(Math.max(8, y)) };
      }
      const w = new WebviewWindow('mini', {
        url: at ? '/?mini=1&pop=1' : '/?mini=1',
        ...pos,
        title: 'Tarn',
        width: 360,
        height: 110,
        resizable: false,
        decorations: false,
        // Transparent so the rounded card has no square window corners behind it.
        transparent: true,
        alwaysOnTop: true,
        shadow: false,
      });
      w.once('tauri://error', (e) => fail(e.payload, "Couldn't open the mini player"));
    } catch (e) { fail(e, "Couldn't open the mini player"); }
  }

  // Scrobbling (Last.fm / ListenBrainz): the tracker watches playback state.
  const scrobbler = new Scrobbler();
  $effect(() => {
    scrobbler.update(player.track ?? null, player.position, player.duration, player.playing);
  });

  async function exportCollection() {
    const c = collection;
    if (!c?.tracks.length) return;
    const csv = tracksToCsv(c.tracks);
    try {
      await navigator.clipboard.writeText(csv);
      toast(tr('Copied {n} songs as CSV', { n: c.tracks.length }), 'info', 2800);
    } catch {
      toast(tr("Couldn't copy to the clipboard"), 'error', 2800);
    }
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
    const name = library?.playlists?.find((p) => p.id === pid)?.name;
    toast(name ? tr('Added to {name}', { name }) : tr('Added to playlist'), 'info', 2000);
    try { await api.playlistAdd(pid, vid); } catch (e) { fail(e, "Couldn't add to this playlist", 'playlist'); }
  }

  /** Turn the current queue into a new playlist on the account. */
  async function saveQueueAsPlaylist() {
    const ids = [...new Set(player.queue.map((t) => t.id).filter(Boolean))];
    if (!ids.length) return;
    if (needAccount('playlist')) return;
    const title = prompt('Playlist name', 'Queue');
    if (!title) return;
    toast(tr('Saving {n} songs…', { n: ids.length }), 'info', 2000);
    try {
      const pid = await api.playlistCreate(title);
      for (const id of ids) await api.playlistAdd(pid, id);
      toast(tr('Saved “{title}” ({n} songs)', { title, n: ids.length }), 'info', 2800);
      loadLibrary();
    } catch (e) { fail(e, "Couldn't save the queue", 'playlist'); }
  }

  function goArtist(id: string, name?: string) {
    if (id) go({ id: 'artist', artistId: id });
    else if (name) { searchQ = name; runSearch(name); }
  }

  // Smooth playback clock (ms) for lyrics: the backend position ticks
  // coarsely, so extrapolate between ticks with rAF while lyrics are visible.
  let lyMs = $state(0);
  let lyAnchor = { pos: 0, t: 0 };
  $effect(() => {
    const pos = player.position;
    const now = performance.now();
    const est = lyAnchor.pos + (player.playing ? (now - lyAnchor.t) / 1000 : 0);
    const err = pos - est;
    if (!player.playing || Math.abs(err) > 0.35) {
      // Seek, pause or big drift: snap.
      lyAnchor = { pos, t: now };
      lyMs = pos * 1000;
    } else {
      // Normal tick: absorb a fraction of the error so the fill never jumps.
      lyAnchor = { pos: est + err * 0.2, t: now };
    }
  });
  $effect(() => {
    if (!lyricsOn || !lyrics?.synced || !player.playing) return;
    let raf = 0;
    const tick = () => {
      lyMs = (lyAnchor.pos + (performance.now() - lyAnchor.t) / 1000) * 1000;
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });
  let activeLyric = $derived.by(() => {
    if (!lyrics?.synced) return -1;
    const ms = lyMs;
    let idx = -1;
    for (let i = 0; i < lyrics.lines.length; i++) {
      const s = lyrics.lines[i].start_ms;
      if (s != null && s <= ms) idx = i;
    }
    return idx;
  });
  /** People sing ~12 letters/s; don't stretch the fill across long pauses. */
  const sungMs = (ln: { text: string }) => Math.max(900, ln.text.length * 85);
  /** Long instrumental gaps: [startMs, endMs] before line i (null = none). */
  let lyGaps = $derived.by(() => {
    const out: ({ a: number; b: number } | null)[] = [];
    const ls = lyrics?.synced ? lyrics.lines : [];
    let prevEnd = 0;
    for (let i = 0; i < ls.length; i++) {
      const st = ls[i].start_ms;
      if (st == null) { out.push(null); continue; }
      const gapStart = i === 0 ? 0 : prevEnd + 400;
      out.push(st - gapStart >= (i === 0 ? 5000 : 4500) ? { a: gapStart, b: st } : null);
      prevEnd = st + sungMs(ls[i]);
    }
    return out;
  });
  /** Index of the line a gap is currently waiting for, or -1. */
  let gapNow = $derived.by(() => {
    if (!settings.lyrics_dots) return -1;
    for (let i = 0; i < lyGaps.length; i++) {
      const g = lyGaps[i];
      if (g && lyMs >= g.a && lyMs < g.b) return i;
    }
    return -1;
  });
  let gapP = $derived(gapNow < 0 ? 0 : (lyMs - lyGaps[gapNow]!.a) / (lyGaps[gapNow]!.b - lyGaps[gapNow]!.a));

  /** 0..1 progress through the active line, estimated from its length. */
  let lineProgress = $derived.by(() => {
    if (!lyrics?.synced || activeLyric < 0) return 0;
    const ln = lyrics.lines[activeLyric];
    const start = ln.start_ms ?? 0;
    let nextStart = Infinity;
    for (let j = activeLyric + 1; j < lyrics.lines.length; j++) {
      const n = lyrics.lines[j].start_ms;
      if (n != null) { nextStart = n; break; }
    }
    const dur = Math.min(nextStart - start, sungMs(ln));
    if (!isFinite(dur) || dur <= 0) return 1;
    return Math.min(1, Math.max(0, (lyMs - start) / dur));
  });
  /** Words of the active line with each letter's global index. */
  let activeWords = $derived.by(() => {
    if (activeLyric < 0 || !lyrics) return { words: [] as { chars: string[]; off: number }[], n: 0 };
    const text = lyrics.lines[activeLyric].text.trim();
    let off = 0;
    const words = text.split(/\s+/).filter(Boolean).map((w) => {
      const chars = Array.from(w);
      const o = off;
      off += chars.length + 1;
      return { chars, off: o };
    });
    return { words, n: Math.max(1, off - 1) };
  });
  /** Lyrics panel is active on fullscreen (kept across track changes). */
  let npLyricsMode = $derived(expanded && lyricsOn);
  /** Synced karaoke column is ready to show. */
  let npLyricsOn = $derived(
    npLyricsMode && lyricsOk && !!lyrics?.synced && (lyrics?.lines.length ?? 0) > 0,
  );
  let npQueueOn = $derived(expanded && queueOn);
  let npLyricsEl: HTMLDivElement | null = $state(null);

  let scrollRaf = 0;
  /** The element to keep centred: the gap dots during a gap, else the active line. */
  function focusEl(root: ParentNode): HTMLElement | null {
    return (gapNow >= 0
      ? root.querySelector(`.ly-gap[data-g="${gapNow}"]`)
      : root.querySelector(`.ly[data-i="${activeLyric}"]`)) as HTMLElement | null;
  }
  // While the fullscreen chrome opens/closes the column resizes for 0.6s;
  // keep the active line pinned to the centre instead of chasing it.
  $effect(() => {
    const el = npLyricsEl;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      const line = focusEl(el);
      if (!line) return;
      cancelAnimationFrame(scrollRaf);
      el.scrollTop = Math.max(0, line.offsetTop - el.clientHeight / 2 + line.offsetHeight / 2);
    });
    ro.observe(el);
    return () => ro.disconnect();
  });
  function easeScroll(el: HTMLElement, to: number) {
    cancelAnimationFrame(scrollRaf);
    const from = el.scrollTop;
    const dist = to - from;
    if (Math.abs(dist) < 1) return;
    const dur = Math.min(900, 450 + Math.abs(dist) * 0.5);
    const t0 = performance.now();
    const step = (now: number) => {
      const k = Math.min(1, (now - t0) / dur);
      el.scrollTop = from + dist * (1 - Math.pow(1 - k, 4));
      if (k < 1) scrollRaf = requestAnimationFrame(step);
    };
    scrollRaf = requestAnimationFrame(step);
  }

  $effect(() => {
    if (!lyrics?.synced || (activeLyric < 0 && gapNow < 0)) return;
    const i = activeLyric;
    void gapNow;
    if (npLyricsOn && npLyricsEl) {
      const line = focusEl(npLyricsEl);
      if (!line) return;
      const top = line.offsetTop - npLyricsEl.clientHeight / 2 + line.offsetHeight / 2;
      easeScroll(npLyricsEl, Math.max(0, top));
      return;
    }
    if (!expanded) {
      const el = focusEl(document.querySelector('.panel .lyrics') ?? document);
      el?.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }
  });

  function fmtCache(n: number) {
    if (n < 1024) return `${n} B`;
    if (n < 1024 ** 2) return `${(n / 1024).toFixed(0)} KB`;
    if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(n >= 10 * 1024 ** 2 ? 0 : 1)} MB`;
    if (n < 1024 ** 4) return `${(n / 1024 ** 3).toFixed(n >= 10 * 1024 ** 3 ? 0 : 1)} GB`;
    if (n < 1024 ** 5) return `${(n / 1024 ** 4).toFixed(1)} TB`;
    if (n < 1024 ** 6) return `${(n / 1024 ** 5).toFixed(1)} PB`;
    return `${(n / 1024 ** 6).toFixed(1)} EB`;
  }
  type CacheUnit = 'MB' | 'GB' | 'TB' | 'PB' | 'EB';
  const CACHE_UNIT_MB: Record<CacheUnit, number> = {
    MB: 1,
    GB: 1024,
    TB: 1024 ** 2,
    PB: 1024 ** 3,
    EB: 1024 ** 4,
  };
  const CACHE_PRESETS = $derived<{ value: string; label: string }[]>([
    { value: '0', label: tr('Off') },
    { value: '256', label: '256 MB' },
    { value: '512', label: '512 MB' },
    { value: '1024', label: '1 GB' },
    { value: '2048', label: '2 GB' },
    { value: '4096', label: '4 GB' },
    { value: '5120', label: '5 GB' },
    { value: '10240', label: '10 GB' },
    { value: '15360', label: '15 GB' },
    { value: '20480', label: '20 GB' },
    { value: '25600', label: '25 GB' },
    { value: String(1024 ** 2), label: '1 TB' },
    { value: String(1024 ** 3), label: '1 PB' },
    { value: String(1024 ** 4), label: '1 EB' },
    { value: 'custom', label: tr('Custom…') },
  ]);
  let cacheMode = $state<'preset' | 'custom'>('preset');
  let cacheCustomAmt = $state('8');
  let cacheCustomUnit = $state<CacheUnit>('GB');
  const cacheSelect = $derived(
    CACHE_PRESETS.some((p) => p.value !== 'custom' && Number(p.value) === settings.cache_max_mb)
      ? String(settings.cache_max_mb)
      : 'custom',
  );
  function syncCacheCustomFromSettings() {
    const mb = Number(settings.cache_max_mb) || 0;
    const preset = CACHE_PRESETS.some((p) => p.value !== 'custom' && Number(p.value) === mb);
    cacheMode = preset ? 'preset' : 'custom';
    if (preset) return;
    const units: CacheUnit[] = ['EB', 'PB', 'TB', 'GB', 'MB'];
    for (const u of units) {
      const f = CACHE_UNIT_MB[u];
      if (mb >= f && mb % f === 0) {
        cacheCustomAmt = String(mb / f);
        cacheCustomUnit = u;
        return;
      }
    }
    cacheCustomAmt = String(mb);
    cacheCustomUnit = 'MB';
  }
  async function applyCachePreset(v: string) {
    if (v === 'custom') {
      cacheMode = 'custom';
      if (!cacheCustomAmt) {
        cacheCustomAmt = '8';
        cacheCustomUnit = 'GB';
      }
      await applyCacheCustom();
      return;
    }
    cacheMode = 'preset';
    const mb = Number(v);
    settings.cache_max_mb = mb;
    settings = { ...settings };
    await api.setSettings(settings);
    await refreshCacheStats();
  }
  async function applyCacheCustom() {
    let n = Number(String(cacheCustomAmt).replace(',', '.'));
    if (!Number.isFinite(n) || n < 0) return;
    const mb = Math.min(
      Math.round(n * CACHE_UNIT_MB[cacheCustomUnit]),
      Number.MAX_SAFE_INTEGER,
    );
    cacheMode = 'custom';
    settings.cache_max_mb = mb;
    settings = { ...settings };
    await api.setSettings(settings);
    await refreshCacheStats();
  }
  async function refreshCacheStats() {
    try {
      const [used, max] = await api.cacheStats();
      cacheUsed = used;
      cacheMax = max;
    } catch {}
  }
  let analysisCount = $state(0);
  async function refreshAnalysisCount() {
    try {
      analysisCount = await api.analysisCount();
    } catch {}
  }
  async function clearAnalysis() {
    try {
      await api.clearAnalysis();
    } catch {}
    await refreshAnalysisCount();
  }
  $effect(() => {
    if (view.id !== 'settings') return;
    void refreshCacheStats();
    void refreshAnalysisCount();
  });

  // Audio output devices (desktop picker in Settings → Sound).
  let outputs = $state<{ devices: string[]; default: string | null; current: string | null }>({ devices: [], default: null, current: null });
  async function refreshOutputs() {
    try { outputs = await api.outputDevices(); } catch {}
  }
  $effect(() => {
    if (view.id !== 'settings' || isPhone) return;
    void refreshOutputs();
    const t = setInterval(refreshOutputs, 5000);
    return () => clearInterval(t);
  });

  let navOn = (id: string) => view.id === id;

  let winFocused = $state(true);

  function armNpHide() {
    if (npHideTimer) clearTimeout(npHideTimer);
    // Unfocused window: keep fullscreen chrome hidden (same as idle fade).
    if (!winFocused) {
      npShow = false;
      return;
    }
    npShow = true;
    // Phones: keep the controls on screen (no hover to bring them back).
    if (typeof matchMedia === 'function' && matchMedia('(max-width: 700px)').matches) return;
    // Hide after the mouse has been still for a while, even when it rests on
    // the lyrics or the player bar. Any movement shows it again.
    if (expanded) {
      npHideTimer = setTimeout(() => {
        if (expanded && winFocused) npShow = false;
      }, 2500);
    }
  }

  function onAppMove() {
    if (expanded && winFocused) armNpHide();
  }

  $effect(() => {
    if (expanded) armNpHide();
    else {
      npShow = true;
      if (npHideTimer) clearTimeout(npHideTimer);
    }
  });

  /** Real OS fullscreen for the whole window (F11). */
  async function toggleWindowFullscreen() {
    try {
      const w = getCurrentWindow();
      await w.setFullscreen(!(await w.isFullscreen()));
    } catch { /* not permitted / unsupported */ }
  }

  onMount(() => {
    const win = getCurrentWindow();
    const unsubs: Array<() => void> = [];
    listen<{ x: number; y: number }>('tray-mini', (e) => void openMini(e.payload))
      .then((u) => unsubs.push(u))
      .catch(() => {});
    win
      .onFocusChanged(({ payload: focused }) => {
        winFocused = focused;
        if (!expanded) return;
        if (focused) armNpHide();
        else {
          npShow = false;
          if (npHideTimer) clearTimeout(npHideTimer);
        }
      })
      .then((u) => unsubs.push(u))
      .catch(() => {});
    win.isFocused().then((f) => {
      winFocused = f;
      if (expanded && !f) npShow = false;
    }).catch(() => {});
    return () => unsubs.forEach((u) => u());
  });

  $effect(() => {
    const t = player.track;
    const idx = player.index;
    const id = t?.id ?? '';
    if (!expanded || !id) {
      npPrevId = id;
      npPrevIndex = idx;
      return;
    }
    if (npPrevId && id !== npPrevId) {
      npSlide = idx < npPrevIndex ? 'prev' : 'next';
    }
    npPrevId = id;
    npPrevIndex = idx;
  });
</script>

{#snippet karaoke()}
  <span class="kw" style:--p={lineProgress} style:--n={activeWords.n}>
    {#each activeWords.words as w}<span class="kword">{#each w.chars as ch, ci}<span class="kc" style:--i={w.off + ci}>{ch}</span>{/each}</span>{' '}{/each}
  </span>
{/snippet}

{#snippet gapDots(i: number)}
  {#if lyGaps[i] && settings.lyrics_dots}
    <div class="ly-gap" class:on={gapNow === i} data-g={i} style:--gp={gapNow === i ? gapP : 0} aria-hidden="true"><i></i><i></i><i></i></div>
  {/if}
{/snippet}


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
    <!-- Drag strip stays outside the opacity layer (WebView2). -->
    <div
      class="tb-drag-hit"
      class:mac={isMac}
      class:off={expanded && !npShow}
      data-tauri-drag-region
    ></div>
    <div class="tb-fade">
      <TitleBar canBack={backStack.length > 0} canFwd={fwdStack.length > 0} onBack={goBack} onFwd={goFwd} />
    </div>
  </div>
  <aside class="sidebar" class:is-hidden={expanded}>
    <div class="brand">
      <img src="/logo.png" width="44" height="44" alt="" />
      <span class="word">Tarn</span>
    </div>
    <nav class="nav">
      <button class="nav-item {navOn('home') ? 'on' : ''}" onclick={() => go({ id: 'home' })} type="button">
        <span class="ic"><Hi icon={Home01Icon} size={24} /></span> {tr('Home')}
      </button>
      <button class="nav-item {navOn('explore') || view.id === 'charts' || view.id === 'new-releases' || view.id === 'genre' ? 'on' : ''}" onclick={() => go({ id: 'explore' })} type="button">
        <span class="ic"><Hi icon={Compass01Icon} size={24} /></span> {tr('Explore')}
      </button>
      <button class="nav-item {navOn('search') ? 'on' : ''}" onclick={() => go({ id: 'search' })} type="button">
        <span class="ic"><Hi icon={Search01Icon} size={24} /></span> {tr('Search')}
      </button>
      <button class="nav-item {navOn('library') || view.id === 'history' ? 'on' : ''}" onclick={() => go({ id: 'library' })} type="button">
        <span class="ic"><Hi icon={LibraryIcon} size={24} /></span> {tr('Library')}
      </button>
      <button class="nav-item {navOn('downloads') ? 'on' : ''}" onclick={() => go({ id: 'downloads' })} type="button">
        <span class="ic"><Hi icon={Download04Icon} size={24} /></span> {tr('Downloads')}
        {#if dlBusy}<span class="nav-dot" title={tr('Downloading')}></span>{/if}
      </button>
      <button class="nav-item {navOn('stats') ? 'on' : ''}" onclick={() => go({ id: 'stats' })} type="button">
        <span class="ic"><Hi icon={ChartColumnIcon} size={24} /></span> {tr('Stats')}
      </button>
      <button class="nav-item {navOn('smart') ? 'on' : ''}" onclick={() => go({ id: 'smart' })} type="button">
        <span class="ic"><Hi icon={MagicWand01Icon} size={24} /></span> Smart playlists
      </button>
    </nav>
    <div class="side-scroll">
      {#if auth.signed_in && !off.offline && !library?.playlists?.length && (libLoading || libPartial || !library)}
        <div class="side-label">{tr('Playlists')}</div>
        {#each { length: 7 } as _, i (i)}
          <div class="side-pl-sk" aria-hidden="true">
            <div class="shimmer sk-sq"></div>
            <div class="shimmer sk-ln" style="width:{[70, 55, 80, 62, 48, 74, 58][i]}%"></div>
          </div>
        {/each}
      {:else if auth.signed_in && (sidePlaylists.length > 0 || sideQ || (!off.offline && library?.playlists?.length))}
        <div class="side-label">{tr('Playlists')}</div>
        {#if (library?.playlists?.length ?? 0) > 7}
          <FilterBar compact id="side-filter" bind:value={sideQ} placeholder={tr('Filter playlists')} />
        {/if}
        {#each sidePlaylists as p, i (p.id)}
          <button class="side-pl pl-in" style="--i:{Math.min(i, 12)}" type="button" onclick={() => go({ id: 'playlist', playlistId: p.id })} oncontextmenu={(e) => ctxColl(e, { kind: 'playlist', id: p.id, title: p.name, cover: p.cover })}>
            <PlaylistCover id={p.id} name={p.name} cover={p.cover} covers={p.covers} size={36} alt="" />
            <span class="clamp one">{p.name}</span>
          </button>
        {/each}
      {/if}
    </div>
    <div class="side-foot">
      <button class="nav-item {navOn('settings') ? 'on' : ''}" onclick={() => go({ id: 'settings' })} type="button">
        <span class="ic"><Hi icon={Settings02Icon} size={22} /></span> {tr('Settings')}
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
              {#if auth.mock}<span class="badge">{tr('Preview')}</span>{/if}
            </div>
            {#if auth.account.handle}<div class="hd">{auth.account.handle}</div>{/if}
          </div>
        </div>
      {:else}
        <button class="acct" type="button" onclick={() => api.openLogin()}>
          <span class="av"><Hi icon={Login03Icon} size={22} /></span>
          <div class="nm">{tr('Sign in')}</div>
        </button>
      {/if}
    </div>
  </aside>

  <main class="main {sidePanelOn && !expanded ? 'with-panel' : ''}" class:is-hidden={expanded} bind:this={mainEl}>
    <div class="page">
      {#if off.offline}
        <div class="off-banner" role="status">
          <Hi icon={WifiOff01Icon} size={18} />
          <span>{off.mode ? tr('Offline mode is on - only downloaded songs play.') : tr("You're offline - only downloaded songs play.")}</span>
          {#if off.mode}
            <button class="btn ghost sm" type="button" onclick={() => api.setOfflineMode(false)}>{tr('Go online')}</button>
          {/if}
        </div>
      {/if}

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
          <ShelfRow shelf={s} onPlayItems={playTracks} onOpen={openItem} onContext={contextItem} onPlayCollection={(it) => playCollection(it, false)} />
        {/each}

      {:else if view.id === 'explore'}
        <SearchBar
          bind:value={searchQ}
          onSubmit={(q) => runSearch(q)}
          onPlay={(t, queue) => playTracks(queue, Math.max(0, queue.findIndex((x) => x.id === t.id)))}
          onOpen={openItem}
          onContext={contextItem}
        />
        <h1 class="page-title">{tr('Explore')}</h1>
        <div class="explore-links">
          <button type="button" onclick={() => go({ id: 'new-releases' })}><Hi icon={Album02Icon} size={24} /> {tr('New releases')}</button>
          <button type="button" onclick={() => go({ id: 'charts' })}><Hi icon={MusicNote01Icon} size={24} /> {tr('Charts')}</button>
          <button type="button" onclick={() => document.getElementById('moods')?.scrollIntoView({ behavior: 'smooth' })}><Hi icon={Compass01Icon} size={24} /> {tr('Moods & genres')}</button>
        </div>
        {#if loading && !explore}<Skeleton kind="explore" />{/if}
        {#each (explore?.shelves ?? []).filter((s) => s.kind !== 'moods' && !s.title.toLowerCase().includes('mood')) as s}
          <ShelfRow shelf={s} onPlayItems={playTracks} onOpen={openItem} onContext={contextItem} onPlayCollection={(it) => playCollection(it, false)} />
        {/each}
        {#if explore?.moods?.length}
          <section class="shelf" id="moods">
            <div class="shelf-head"><h2 class="shelf-title">{tr('Moods & genres')}</h2></div>
            <div class="mood-grid">
              {#each explore.moods as m}
                <button class="mood" style="background:{m.color || '#3a4a5c'}" type="button" onclick={() => openItem(m)}>{m.title}</button>
              {/each}
            </div>
          </section>
        {/if}

      {:else if view.id === 'charts' || view.id === 'new-releases'}
        <h1 class="page-title">{view.id === 'charts' ? tr('Charts') : tr('New releases')}</h1>
        {#if loading && !(view.id === 'charts' ? charts : releases)}<Skeleton kind="home" />{/if}
        {#each (view.id === 'charts' ? charts : releases)?.shelves ?? [] as s}
          <ShelfRow shelf={s} onPlayItems={playTracks} onOpen={openItem} onContext={contextItem} onPlayCollection={(it) => playCollection(it, false)} />
        {/each}

      {:else if view.id === 'search'}
        <SearchBar
          bind:value={searchQ}
          autofocus
          onSubmit={(q) => runSearch(q)}
          onPlay={(t, queue) => playTracks(queue, Math.max(0, queue.findIndex((x) => x.id === t.id)))}
          onOpen={openItem}
          onContext={contextItem}
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
              oncontextmenu={(e) => contextItem(e, top)}
              role="group"
            >
              <button
                class="top-art"
                type="button"
                title={tr('Play')}
                onclick={() => {
                  if (top.kind === 'track' || top.kind === 'video') playOne(itemToTrack(top));
                  else playCollection(top, false);
                }}
              >
                <Cover src={top.cover} size={160} round={top.kind === 'artist'} alt="" />
                <span class="play-ov"><Hi icon={PlayIcon} size={40} /></span>
              </button>
              <div>
                <button class="top-meta" type="button" onclick={() => openItem(top)}>
                  <div class="k">{top.kind}</div>
                  <div class="ttl">{top.title}</div>
                  <div class="sub">{top.subtitle || artistsLine(top.artists)}</div>
                </button>
                <div class="acts">
                  {#if top.kind === 'track' || top.kind === 'video'}
                    <button class="btn" type="button" onclick={() => playOne(itemToTrack(top))}><Hi icon={PlayIcon} size={18} /> {tr('Play')}</button>
                    <button class="btn ghost" type="button" onclick={() => playTopShuffled(top)}><Hi icon={ShuffleIcon} size={18} /> {tr('Shuffle')}</button>
                    <button class="btn ghost" type="button" onclick={() => startRadio(top.id)}><Hi icon={Radio02Icon} size={18} /> {tr('Radio')}</button>
                  {:else}
                    <button class="btn" type="button" onclick={() => playCollection(top, false)}><Hi icon={PlayIcon} size={18} /> {tr('Play')}</button>
                    <button class="btn ghost" type="button" onclick={() => playCollection(top, true)}><Hi icon={ShuffleIcon} size={18} /> {tr('Shuffle')}</button>
                    <button class="btn ghost" type="button" onclick={() => openItem(top)}>{tr('Open')}</button>
                  {/if}
                </div>
              </div>
            </div>
          {/if}
          {#if searchTab === 'top' || searchTab === 'songs'}
            {#if searchTab === 'top'}<h2 class="shelf-title" style="margin-bottom:12px">{tr('Songs')}</h2>{/if}
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
              {#if searchTab === 'top'}<h2 class="shelf-title" style="margin:28px 0 12px">{tr('Albums')}</h2>{/if}
              <div class="row">
                {#each results.albums as a}
                  <div class="card" role="group">
                    <button class="art" type="button" title={tr('Play')} onclick={() => playAlbumId(a.id, a.cover)}>
                      <Cover src={a.cover} size={152} alt="" />
                      <span class="play-ov"><Hi icon={PlayIcon} size={36} /></span>
                    </button>
                    <button class="c-a" type="button" onclick={() => go({ id: 'album', albumId: a.id })}>
                      <div class="title">{a.name}</div>
                      <div class="sub">{a.subtitle || a.album_type || ''}{a.year ? ` · ${a.year}` : ''}</div>
                    </button>
                  </div>
                {/each}
              </div>
            {/if}
          {/if}
          {#if searchTab === 'top' || searchTab === 'artists'}
            {#if results.artists.length}
              {#if searchTab === 'top'}<h2 class="shelf-title" style="margin:28px 0 12px">{tr('Artists')}</h2>{/if}
              <div class="row">
                {#each results.artists as a}
                  <div class="card artist" role="group">
                    <button class="art" type="button" title={tr('Play')} onclick={() => playArtistId(a.id, a.cover)}>
                      <Cover src={a.cover} size={132} round alt="" />
                      <span class="play-ov"><Hi icon={PlayIcon} size={28} /></span>
                    </button>
                    <button class="c-a" type="button" onclick={() => go({ id: 'artist', artistId: a.id })}>
                      <div class="title">{a.name}</div>
                    </button>
                  </div>
                {/each}
              </div>
            {/if}
          {/if}
          {#if searchTab === 'top' || searchTab === 'playlists'}
            {#if results.playlists.length}
              {#if searchTab === 'top'}<h2 class="shelf-title" style="margin:28px 0 12px">{tr('Playlists')}</h2>{/if}
              <div class="row">
                {#each results.playlists as p}
                  <div class="card" role="group">
                    <button class="art" type="button" title={tr('Play')} onclick={() => playPlaylistId(p.id, p.cover, p.covers)}>
                      <PlaylistCover id={p.id} name={p.name} cover={p.cover} covers={p.covers} size={152} alt="" />
                      <span class="play-ov"><Hi icon={PlayIcon} size={36} /></span>
                    </button>
                    <button class="c-a" type="button" onclick={() => go({ id: 'playlist', playlistId: p.id })}>
                      <div class="title">{p.name}</div>
                      <div class="sub">{p.subtitle || ''}</div>
                    </button>
                  </div>
                {/each}
              </div>
            {/if}
          {/if}
          {#if searchTab === 'community'}
            <div class="row">
              {#each results.community as p}
                <div class="card" role="group">
                  <button class="art" type="button" title={tr('Play')} onclick={() => playPlaylistId(p.id, p.cover, p.covers)}>
                    <PlaylistCover id={p.id} name={p.name} cover={p.cover} covers={p.covers} size={152} alt="" />
                    <span class="play-ov"><Hi icon={PlayIcon} size={36} /></span>
                  </button>
                  <button class="c-a" type="button" onclick={() => go({ id: 'playlist', playlistId: p.id })}>
                    <div class="title">{p.name}</div>
                    <div class="sub">{p.subtitle || ''}</div>
                  </button>
                </div>
              {/each}
            </div>
          {/if}
        {/if}

      {:else if view.id === 'library'}
        <h1 class="page-title">{tr('Library')} {#if auth.mock}<span class="badge">{tr('Preview')}</span>{/if}</h1>
        {#if !auth.signed_in}
          <div class="lib-guest">
            <img src="/logo.png" width="80" height="80" alt="" />
            <p>{tr('Sign in to see your playlists, likes and albums')}</p>
            <button class="btn" type="button" onclick={() => api.openLogin()}>{tr('Sign in')}</button>
          </div>
          {#if recents.length}
            <h2 class="shelf-title" style="margin:12px 0 12px">{tr('Recently played')}</h2>
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
            <h2 class="shelf-title" style="margin:28px 0 12px">{tr('Liked songs')}</h2>
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
          {#if (libLoading && !library) || (libPartial && !library?.liked?.length)}
            <Skeleton kind="page" />
          {/if}
          <div class="tabs" style="padding-top:0">
            <button class="tab on" type="button" onclick={() => go({ id: 'history' })}><Hi icon={HistoryIcon} size={18} /> {tr('History')}</button>
          </div>
          {#if library && (library.liked.length || library.playlists.length || library.albums.length || library.artists.length)}
            <div class="lib-filter">
              <FilterBar bind:value={filterQ} placeholder={tr('Search your library  (Ctrl+F)')}
                count={`${fLiked.length + fPlaylists.length + fAlbums.length + fArtists.length} results`} />
            </div>
          {/if}
          {#if filterQ && !fLiked.length && !fPlaylists.length && !fAlbums.length && !fArtists.length && library}
            <div class="no-match">Nothing in your library matches “{filterQ}”</div>
          {/if}
          {#if auth.signed_in && !off.offline}
            <div style="margin-top:20px">
              <button class="btn ghost" type="button" onclick={() => (importOpen = true)}>{tr('Import playlist')}</button>
            </div>
          {/if}
          {#if fPlaylists.length}
            <h2 class="shelf-title" style="margin:28px 0 12px">{tr('Playlists')}</h2>
            <div class="row">
              {#each fPlaylists as p (p.id)}
                <div class="card" role="group" oncontextmenu={(e) => ctxColl(e, { kind: 'playlist', id: p.id, title: p.name, cover: p.cover })}>
                  <button class="art" type="button" title={tr('Play')} onclick={() => playPlaylistId(p.id, p.cover, p.covers)}>
                    <PlaylistCover id={p.id} name={p.name} cover={p.cover} covers={p.covers} size={152} alt="" />
                    <span class="play-ov"><Hi icon={PlayIcon} size={36} /></span>
                  </button>
                  <button class="c-a" type="button" onclick={() => go({ id: 'playlist', playlistId: p.id })}>
                    <div class="title">{p.name}</div>
                    <div class="sub">{p.track_count ? `${p.track_count}` : ''}</div>
                  </button>
                </div>
              {/each}
            </div>
          {/if}
          {#if fAlbums.length}
            <h2 class="shelf-title" style="margin:28px 0 12px">{tr('Albums')}</h2>
            <div class="row">
              {#each fAlbums as a (a.id)}
                <div class="card" role="group" oncontextmenu={(e) => ctxColl(e, { kind: 'album', id: a.id, title: a.name, cover: a.cover, subtitle: a.subtitle })}>
                  <button class="art" type="button" title={tr('Play')} onclick={() => playAlbumId(a.id, a.cover)}>
                    <Cover src={a.cover} size={152} alt="" />
                    <span class="play-ov"><Hi icon={PlayIcon} size={36} /></span>
                  </button>
                  <button class="c-a" type="button" onclick={() => go({ id: 'album', albumId: a.id })}>
                    <div class="title">{a.name}</div>
                    <div class="sub">{a.subtitle || ''}</div>
                  </button>
                </div>
              {/each}
            </div>
          {/if}
          {#if fArtists.length}
            <h2 class="shelf-title" style="margin:28px 0 12px">{tr('Artists')}</h2>
            <div class="row">
              {#each fArtists as a (a.id)}
                <div class="card artist" role="group" oncontextmenu={(e) => ctxColl(e, { kind: 'artist', id: a.id, title: a.name, cover: a.cover })}>
                  <button class="art" type="button" title={tr('Play')} onclick={() => playArtistId(a.id, a.cover)}>
                    <Cover src={a.cover} size={132} round alt="" />
                    <span class="play-ov"><Hi icon={PlayIcon} size={28} /></span>
                  </button>
                  <button class="c-a" type="button" onclick={() => go({ id: 'artist', artistId: a.id })}>
                    <div class="title">{a.name}</div>
                  </button>
                </div>
              {/each}
            </div>
          {/if}
          {#if fLiked.length}
            <div class="liked-h" style="margin-top:28px">
              <h2 class="shelf-title" style="margin:0">{tr('Liked')}{filterQ ? ` · ${tr('{a} of {b}', { a: fLiked.length, b: library?.liked.length ?? 0 })}` : ''}</h2>
              {#if !off.offline || sourceInfo('liked')}
                {@const ls = sourceInfo('liked')}
                <button class="btn ghost sm" type="button" onclick={() => toggleLikedDownload()}>
                  {#if !ls}<Hi icon={Download04Icon} size={16} /> {tr('Download')}
                  {:else if ls.done >= ls.total}<Hi icon={CheckmarkCircle02Icon} size={16} /> {tr('Downloaded')}
                  {:else}<Hi icon={Cancel01Icon} size={16} /> Cancel · {ls.done}/{ls.total}{/if}
                </button>
              {/if}
            </div>
            <TrackTable tracks={likedShown} currentId={player.track?.id} playing={player.playing}
              onPlay={(t, i) => playTracks(fLiked, i)}
              onArtist={(id, n) => goArtist(id, n)}
              onAlbum={(id) => go({ id: 'album', albumId: id })}
              onContext={(e, t) => contextTrack(e, t)} />
            {#if fLiked.length > LIKED_PREVIEW}
              <button class="btn ghost more-btn" type="button" onclick={() => (likedAll = !likedAll)}>
                {likedAll ? tr('Show fewer') : tr('Show all {n} songs', { n: fLiked.length })}
              </button>
            {/if}
          {/if}
        {/if}

      {:else if view.id === 'stats'}
        <Stats
          enabled={settings.listening_stats}
          onPlay={(tracks, i) => playTracks(tracks, i)}
          onArtist={(id) => go({ id: 'artist', artistId: id })}
          onAlbum={(id) => go({ id: 'album', albumId: id })}
          onEnable={() => { settings.listening_stats = true; api.setSettings(settings); }}
        />

      {:else if view.id === 'smart'}
        <SmartPlaylists
          liked={library?.liked ?? []}
          currentId={player.track?.id}
          playing={player.playing}
          statsEnabled={settings.listening_stats}
          onPlay={(tracks, i, shuffle) => { void playTracks(tracks, i).then(() => api.setShuffle(!!shuffle)); }}
          onContext={(e, t) => contextTrack(e, t)}
          onArtist={(id, n) => goArtist(id, n)}
          onAlbum={(id) => go({ id: 'album', albumId: id })}
        />

      {:else if view.id === 'downloads'}
        <h1 class="page-title">{tr('Downloads')}</h1>
        <p class="dl-sub">
          {#if off.status?.sources.length}
            {dlDone} of {dlTotal} songs on this device · {fmtBytes(off.status?.bytes ?? 0)}
          {:else}
            {tr('Download playlists, albums and your liked songs to listen without a connection.')}
          {/if}
        </p>
        {#if dlBusy || (off.status?.failed ?? 0) > 0}
          <div class="dl-card">
            <div class="dl-card-top">
              <div class="dl-card-t">
                {#if dlBusy}
                  {off.status?.paused ? 'Paused' : off.offline ? tr('Waiting for a connection') : tr('Downloading')}
                  · {(off.status?.queued ?? 0) + (off.status?.active.length ?? 0)} left
                {:else}
                  {off.status?.failed} couldn't be downloaded
                {/if}
              </div>
              <div class="dl-card-a">
                {#if dlBusy}
                  <button class="btn ghost sm" type="button" onclick={() => api.offlinePause(!off.status?.paused)}>{off.status?.paused ? tr('Resume') : tr('Pause')}</button>
                  <button class="btn ghost sm" type="button" onclick={() => { api.offlineCancel(); toast(tr('Downloads cancelled'), 'info', 2200); }}>{tr('Cancel all')}</button>
                {/if}
                {#if (off.status?.failed ?? 0) > 0}
                  <button class="btn ghost sm" type="button" onclick={() => api.offlineRetry()}>{tr('Retry failed')}</button>
                {/if}
              </div>
            </div>
            {#if off.status?.active.length}
              <div class="dl-now clamp one">{off.status.active.map((a) => a.title).filter(Boolean).join(' · ')}</div>
            {/if}
            <div class="dl-bar"><span style="width:{dlTotal ? Math.round((dlDone / dlTotal) * 100) : 0}%"></span></div>
          </div>
        {/if}
        {#if off.status?.sources.length}
          <h2 class="shelf-title" style="margin:20px 0 12px">{tr('Collections')}</h2>
          <div class="row">
            {#each off.status.sources as src (src.id)}
              <div class="card" role="group" oncontextmenu={(e) => src.kind !== 'liked' && ctxColl(e, { kind: src.kind === 'album' ? 'album' : 'playlist', id: src.id, title: src.name, cover: src.cover })}>
                <button class="art" type="button" title={tr('Play')} onclick={async () => {
                  const c = await api.offlineCollection(src.id);
                  if (c?.tracks.length) playTracks(c.tracks, 0, c.cover);
                }}>
                  {#if src.kind === 'liked'}
                    <PlaylistCover id="LM" name={src.name} cover={null} covers={[]} size={152} alt="" />
                  {:else}
                    <Cover src={src.cover} size={152} alt="" />
                  {/if}
                  <span class="play-ov"><Hi icon={PlayIcon} size={36} /></span>
                  {#if src.done < src.total && !src.failed}
                    <span class="dl-chip">{src.done}/{src.total}</span>
                  {:else if src.done >= src.total}
                    <span class="dl-chip ok"><Hi icon={CheckmarkCircle02Icon} size={14} /></span>
                  {:else}
                    <span class="dl-chip warn">{src.done}/{src.total}</span>
                  {/if}
                </button>
                <div class="c-a">
                  <button class="c-open" type="button" onclick={() => go(src.kind === 'album' ? { id: 'album', albumId: src.id } : src.kind === 'liked' ? { id: 'library' } : { id: 'playlist', playlistId: src.id })}>
                    <div class="title">{src.name}</div>
                    <div class="sub">{src.kind === 'liked' ? 'Liked songs' : src.kind === 'album' ? tr('Album') : tr('Playlist')} · {src.total}</div>
                  </button>
                  {#if src.done < src.total}
                    <button class="dl-rm show" type="button" title={tr('Cancel download')} onclick={() => api.offlineCancel(src.id)}>{tr('Cancel')}</button>
                  {/if}
                  <button class="dl-rm" class:armed={confirmRm === src.id} type="button" title={tr('Remove download')}
                    onclick={() => { if (confirmRm === src.id) { api.offlineRemove(src.id); confirmRm = null; } else { confirmRm = src.id; } }}
                    onblur={() => { if (confirmRm === src.id) confirmRm = null; }}>
                    {#if confirmRm === src.id}Remove?{:else}<Hi icon={Delete02Icon} size={16} />{/if}
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
        {#if dlTracks.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">{tr('Songs')}</h2>
          <div class="lib-filter">
            <FilterBar bind:value={filterQ} placeholder={tr('Search downloads  (Ctrl+F)')} count={`${fDl.length} of ${dlTracks.length}`} />
          </div>
          {#if filterQ && !fDl.length}<div class="no-match">No downloaded songs match “{filterQ}”</div>{/if}
          <TrackTable tracks={fDl} currentId={player.track?.id} playing={player.playing}
            onPlay={(t, i) => playTracks(fDl, i)}
            onArtist={(id, n) => goArtist(id, n)}
            onAlbum={(id) => go({ id: 'album', albumId: id })}
            onContext={(e, t) => contextTrack(e, t)} />
        {:else if !off.status?.sources.length}
          <div class="no-match">{tr('Nothing downloaded yet. Open a playlist or album and press')} <b>{tr('Download')}</b>, or use the button next to <b>{tr('Liked')}</b> in your Library.</div>
        {/if}

      {:else if view.id === 'history'}
        <h1 class="page-title">{tr('History')}</h1>
        {#if history?.length}
          <div class="lib-filter">
            <FilterBar bind:value={filterQ} placeholder={tr('Search history  (Ctrl+F)')} count={`${fHistory.length} of ${history.length}`} />
          </div>
          <TrackTable tracks={fHistory} currentId={player.track?.id} playing={player.playing}
            onPlay={(t, i) => playTracks(fHistory, i)}
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
              <div class="k">{tr('Artist')}</div>
              <h1>{artist.name}</h1>
              {#if artist.subscribers}
                <div class="sub">{tr('{n} subscribers', { n: fmtCount(artist.subscribers) })}</div>
              {/if}
              <div class="hero-acts">
                <button class="btn" type="button" onclick={() => playTracks(artist!.tracks, 0)}><Hi icon={PlayIcon} size={18} /> {tr('Play')}</button>
                <button class="btn ghost" type="button" onclick={async () => { await playTracks(artist!.tracks, 0); api.setShuffle(true); }}><Hi icon={ShuffleIcon} size={18} /> {tr('Shuffle')}</button>
                {#if artist.radio_id}
                  <button class="btn ghost" type="button" onclick={() => api.artistRadio(artist!.radio_id!)}><Hi icon={Radio02Icon} size={18} /> {tr('Radio')}</button>
                {/if}
                {#if auth.signed_in}
                  <button class="btn ghost {artist.subscribed ? 'on' : ''}" type="button" onclick={async () => { if (needAccount('subscribe')) return; const a0 = artist!; const on = !a0.subscribed; artist = { ...a0, subscribed: on }; try { await api.subscribe(a0.id, on); } catch (e) { if (artist?.id === a0.id) artist = { ...artist!, subscribed: !on }; fail(e, "Couldn't subscribe", 'subscribe'); } }}>
                    {artist.subscribed ? tr('Subscribed') : tr('Subscribe')}
                  </button>
                {/if}
              </div>
            </div>
          </div>
        </div>
        {#if artist.tracks.length}
          <h2 class="shelf-title" style="margin-bottom:12px">{tr('Top songs')}</h2>
          <TrackTable tracks={artist.tracks} currentId={player.track?.id} playing={player.playing} showAlbum={true}
            onPlay={(t, i) => playTracks(artist!.tracks, i)}
            onArtist={(id, n) => goArtist(id, n)}
            onAlbum={(id) => go({ id: 'album', albumId: id })}
            onContext={(e, t) => contextTrack(e, t)} />
        {/if}
        {#if artist.albums.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">{tr('Albums')}</h2>
          <div class="row">
            {#each artist.albums as a}
              <div class="card" role="group" oncontextmenu={(e) => ctxColl(e, { kind: 'album', id: a.id, title: a.name, cover: a.cover, subtitle: a.subtitle })}>
                <button class="art" type="button" title={tr('Play')} onclick={() => playAlbumId(a.id, a.cover)}>
                  <Cover src={a.cover} size={152} alt="" />
                  <span class="play-ov"><Hi icon={PlayIcon} size={36} /></span>
                </button>
                <button class="c-a" type="button" onclick={() => go({ id: 'album', albumId: a.id })}>
                  <div class="title">{a.name}</div>
                  <div class="sub">{a.year || a.album_type || ''}</div>
                </button>
              </div>
            {/each}
          </div>
        {/if}
        {#if artist.singles.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">{tr('Singles')}</h2>
          <div class="row">
            {#each artist.singles as a}
              <div class="card" role="group" oncontextmenu={(e) => ctxColl(e, { kind: 'album', id: a.id, title: a.name, cover: a.cover, subtitle: a.subtitle })}>
                <button class="art" type="button" title={tr('Play')} onclick={() => playAlbumId(a.id, a.cover)}>
                  <Cover src={a.cover} size={152} alt="" />
                  <span class="play-ov"><Hi icon={PlayIcon} size={36} /></span>
                </button>
                <button class="c-a" type="button" onclick={() => go({ id: 'album', albumId: a.id })}>
                  <div class="title">{a.name}</div>
                  <div class="sub">{a.year || a.album_type || ''}</div>
                </button>
              </div>
            {/each}
          </div>
        {/if}
        {#if artist.videos.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">{tr('Videos')}</h2>
          <TrackTable tracks={artist.videos} currentId={player.track?.id} playing={player.playing} showAlbum={false}
            onPlay={(t, i) => playTracks(artist!.videos, i)}
            onArtist={(id, n) => goArtist(id, n)}
            onAlbum={(id) => go({ id: 'album', albumId: id })}
            onContext={(e, t) => contextTrack(e, t)} />
        {/if}
        {#if artist.playlists.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">{tr('Featured')}</h2>
          <div class="row">
            {#each artist.playlists as p}
              <button class="card" type="button" onclick={() => go({ id: 'playlist', playlistId: p.id })} oncontextmenu={(e) => ctxColl(e, { kind: 'playlist', id: p.id, title: p.name, cover: p.cover })}>
                <div class="art"><PlaylistCover id={p.id} name={p.name} cover={p.cover} covers={p.covers} size={152} alt="" /></div>
                <div class="c-a"><div class="title">{p.name}</div></div>
              </button>
            {/each}
          </div>
        {/if}
        {#if artist.similar.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">{tr('Similar artists')}</h2>
          <div class="row">
            {#each artist.similar as a}
              <button class="card artist" type="button" onclick={() => go({ id: 'artist', artistId: a.id })} oncontextmenu={(e) => ctxColl(e, { kind: 'artist', id: a.id, title: a.name, cover: a.cover })}>
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
              <div class="k">{collection.album_type || (collection.kind === 'album' ? tr('Album') : tr('Playlist'))}</div>
              <h1>{collection.name}</h1>
              <div class="sub">
                {collection.author || artistsLine(collection.artists)}
                {#if collection.year} · {collection.year}{/if}
                {#if collection.track_count} · {collection.track_count}{/if}
              </div>
              <div class="hero-acts">
                <button class="btn" type="button" onclick={() => playTracks(collection!.tracks, 0, collection!.cover)}><Hi icon={PlayIcon} size={18} /> {tr('Play')}</button>
                <button class="btn ghost" type="button" onclick={async () => { await playTracks(collection!.tracks, 0, collection!.cover); api.setShuffle(true); }}><Hi icon={ShuffleIcon} size={18} /> {tr('Shuffle')}</button>
                <button class="btn ghost" type="button" onclick={() => collection!.tracks[0] && startRadio(collection!.tracks[0].id)}><Hi icon={Radio02Icon} size={18} /> {tr('Radio')}</button>
                {#if colSourceId && (!off.offline || colSrc)}
                  <button class="btn ghost {colSrc && colSrc.done >= colSrc.total ? 'on' : ''}" type="button"
                    title={!colSrc ? 'Download for offline listening' : colSrc.done < colSrc.total ? tr('Cancel download') : tr('Remove download')}
                    onclick={() => toggleDownload()}>
                    {#if !colSrc}
                      <Hi icon={Download04Icon} size={18} /> {tr('Download')}
                    {:else if colSrc.done >= colSrc.total}
                      <Hi icon={CheckmarkCircle02Icon} size={18} /> {tr('Downloaded')}
                    {:else}
                      <Hi icon={Cancel01Icon} size={18} /> Cancel · {colSrc.done}/{colSrc.total}
                    {/if}
                  </button>
                {/if}
                {#if collection.tracks.length}
                  <button class="btn ghost" type="button" title={tr('Copy this list as CSV (title, artist, album, duration, id)')} onclick={exportCollection}>
                    <Hi icon={Link01Icon} size={18} /> {tr('Export CSV')}
                  </button>
                {/if}
                {#if auth.signed_in && !off.offline}
                  <button class="btn ghost {collection.saved ? 'on' : ''}" type="button" onclick={async () => { if (needAccount('save')) return; const c0 = collection!; const s = !c0.saved; collection = { ...c0, saved: s }; try { await api.librarySave(c0.playlist_id || c0.id, s); loadLibrary({ retries: 0 }); } catch (e) { if (collection?.id === c0.id) collection = { ...collection!, saved: !s }; fail(e, "Couldn't save this", 'save'); } }}>
                    <Hi icon={Bookmark02Icon} size={18} /> {collection.saved ? tr('Saved') : tr('Save')}
                  </button>
                {/if}
              </div>
            </div>
          </div>
        </div>
        {#if !collection.tracks.length}
          <div class="no-match">{off.offline ? tr('Nothing from this is downloaded yet.') : tr('This playlist has no songs yet (YouTube Music reports it as empty).')}</div>
        {/if}
        {#if collection.tracks.length > 6}
          <div class="lib-filter">
            <FilterBar bind:value={filterQ} placeholder="Find in this {collection.kind === 'album' ? 'album' : 'playlist'}  (Ctrl+F)" count={`${fCollection.length} of ${collection.tracks.length}`} />
          </div>
        {/if}
        {#if filterQ && !fCollection.length}
          <div class="no-match">No songs match “{filterQ}”</div>
        {/if}
        <TrackTable tracks={fCollection} currentId={player.track?.id} playing={player.playing} showAlbum={collection.kind === 'playlist'} fallbackCover={collection.cover}
          sortable sortId={collection.id}
          onPlay={(t, i, list) => playTracks(list, i, collection!.cover)}
          onArtist={(id, n) => goArtist(id, n)}
          onAlbum={(id) => go({ id: 'album', albumId: id })}
          onContext={(e, t) => contextTrack(e, t)} />
        {#if collection.related.length}
          <h2 class="shelf-title" style="margin:28px 0 12px">{tr('Related')}</h2>
          <div class="row">
            {#each collection.related as p}
              <button class="card" type="button" onclick={() => go({ id: collection!.kind === 'album' ? 'album' : 'playlist', albumId: p.id, playlistId: p.id } as View)} oncontextmenu={(e) => ctxColl(e, { kind: collection!.kind === 'album' ? 'album' : 'playlist', id: p.id, title: p.name, cover: p.cover })}>
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
          <ShelfRow shelf={s} onPlayItems={playTracks} onOpen={openItem} onContext={contextItem} onPlayCollection={(it) => playCollection(it, false)} />
        {/each}

      {:else if view.id === 'settings'}
        <div class="set" bind:this={setEl}>
          <h1 class="page-title">{tr('Settings')}</h1>
          <div class="set-search">
            <Hi icon={Search01Icon} size={18} />
            <input type="search" placeholder={tr('Search settings')} aria-label={tr('Search settings')} bind:value={setQuery} />
            {#if setQuery}
              <button type="button" class="set-clear" aria-label={tr('Clear search')} onclick={() => (setQuery = '')}>×</button>
            {/if}
          </div>
          {#if setEmpty}
            <p class="set-none">No settings match “{setQuery}”.</p>
          {/if}
          <nav class="set-nav" aria-label={tr('Settings sections')}>
            {#each [['general','General'],['playback','Playback'],['transitions','Transitions'],['sound','Sound'],['lyrics','Lyrics & visuals'],['storage','Storage'],['connect','Connections'],['system','System'],['about','About']] as [sid, sl]}
              <button type="button" class="set-chip" onclick={() => document.getElementById('set-' + sid)?.scrollIntoView({ behavior: 'smooth', block: 'start' })}>{tr(sl)}</button>
            {/each}
          </nav>
          <section class="set-sec" id="set-general">
            <div class="set-h"><h2>{tr('General')}</h2><p>{tr('Language, region and sound quality')}</p></div>
            <div class="set-card">
          <div class="set-row">
            <span class="lb"><Hi icon={LanguageCircleIcon} size={20} /> <span>{tr('Interface language')}<span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Menus and buttons in Tarn (song info stays in the YouTube Music language)')}</span></span></span>
            <Dropdown value={settings.ui_language || 'system'} options={localeOptions()} onchange={(v) => { settings.ui_language = v; api.setSettings(settings); setLocale(v); }} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={LanguageCircleIcon} size={20} /> {tr('Content language')}</span>
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
            <span class="lb"><Hi icon={Globe02Icon} size={20} /> {tr('Region')}</span>
            <Dropdown bind:value={settings.region} options={['US','RO','GB','DE','FR','ES','IT','CA','AU','BR','JP','KR','IN','MX','NL','SE','PL'].map((r) => ({ value: r, label: r }))} onchange={() => { api.setSettings(settings); home = null; if (view.id === 'home') loadHome(); }} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={MusicNote01Icon} size={20} /> {tr('Audio quality')}</span>
            <Dropdown bind:value={settings.quality} options={[
              { value: 'high', label: tr('High') },
              { value: 'medium', label: tr('Medium') },
              { value: 'low', label: tr('Low') },
            ]} onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={PaintBoardIcon} size={20} /> Theme</span>
            <Dropdown value={settings.theme ?? 'dark'} options={[
              { value: 'dark', label: 'Dark' },
              { value: 'light', label: 'Light' },
              { value: 'oled', label: 'OLED black' },
              { value: 'system', label: 'Match system' },
            ]} onchange={(v) => { settings.theme = v as Theme; api.setSettings(settings); }} />
          </div>
          <div class="set-row">
            <span class="lb">Accent colour
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{settings.accent_color ? 'Fixed colour' : 'From the cover of the song playing'}</span>
            </span>
            <div class="accent-pick" role="radiogroup" aria-label="Accent colour">
              <button class="acc-sw auto" class:on={!settings.accent_color} title="From cover art" aria-label="From cover art"
                onclick={() => { settings.accent_color = null; api.setSettings(settings); }}></button>
              {#each ACCENTS as c (c)}
                <button class="acc-sw" class:on={settings.accent_color === c} style:background={c} title={c} aria-label={c}
                  onclick={() => { settings.accent_color = c; api.setSettings(settings); }}></button>
              {/each}
              <label class="acc-sw custom" class:on={!!settings.accent_color && !ACCENTS.includes(settings.accent_color)} title="Custom colour"
                style:background={settings.accent_color && !ACCENTS.includes(settings.accent_color) ? settings.accent_color : null}>
                <input type="color" value={settings.accent_color ?? '#e0a45a'}
                  onchange={(e) => { settings.accent_color = (e.currentTarget as HTMLInputElement).value; api.setSettings(settings); }} />
              </label>
            </div>
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={Notification01Icon} size={20} /> Notifications</span>
            <Toggle bind:checked={settings.notifications} label="Notifications" onchange={() => api.setSettings(settings)} />
            <span class="lb"><Hi icon={Notification01Icon} size={20} /> {tr('Notifications')}</span>
            <Toggle bind:checked={settings.notifications} label={tr('Notifications')} onchange={() => api.setSettings(settings)} />
          </div>
            </div>
          </section>
          <section class="set-sec" id="set-playback">
            <div class="set-h"><h2>{tr('Playback')}</h2><p>{tr('What happens when songs start and end')}</p></div>
            <div class="set-card">
          <div class="set-row">
            <span class="lb"><Hi icon={Radio02Icon} size={20} /> {tr('Autoplay')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Continue with similar music when the queue ends')}</span>
            </span>
            <Toggle bind:checked={settings.autoplay} label={tr('Autoplay')} onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={Forward01Icon} size={20} /> Skip non-music sections
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Jump over skits, talking and credits in music videos using community data from SponsorBlock (sends a short hash of the video id)</span>
            </span>
            <Toggle checked={settings.sponsorblock ?? false} label="Skip non-music sections" onchange={(v) => { settings.sponsorblock = v; api.setSettings(settings); }} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={PlayIcon} size={20} /> Resume on start
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Start playing the last session when Tarn opens</span>
            <span class="lb"><Hi icon={PlayIcon} size={20} /> {tr('Resume on start')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Start playing the last session when Tarn opens')}</span>
            </span>
            <Toggle bind:checked={settings.resume_on_start} label={tr('Resume on start')} onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={AudioWave01Icon} size={20} /> Waveform seek bar
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Show the shape of the song in the seek bar (each song is analysed once on this device)</span>
            </span>
            <Toggle checked={!!settings.seek_waveform} label="Waveform seek bar" onchange={(v) => { settings.seek_waveform = v; api.setSettings(settings); }} />
          </div>
            </div>
          </section>
          <section class="set-sec" id="set-transitions">
            <div class="set-h"><h2>{tr('Transitions')}</h2><p>{tr('Crossfade, tempo matching and Auto-DJ')}</p></div>
            <div class="set-card">
          <div class="set-row">
            <span class="lb"><Hi icon={MusicNote01Icon} size={20} /> {tr('Crossfade')}</span>
            <Toggle bind:checked={settings.crossfade} label={tr('Crossfade')} onchange={() => api.setSettings(settings)} />
          </div>
          {#if settings.crossfade}
          <div class="set-row">
            <span class="lb">{tr('Crossfade duration')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{settings.crossfade_seconds.toFixed(1)}s</span>
            </span>
            <div style="width:200px">
              <Slider
                value={settings.crossfade_seconds}
                max={20}
                live={true}
                showTip={true}
                format={(v) => `${v.toFixed(1)}s`}
                oninput={(v) => { settings.crossfade_seconds = Math.round(v * 10) / 10; }}
                onchange={(v) => { settings.crossfade_seconds = Math.round(v * 10) / 10; api.setSettings(settings); }}
              />
            </div>
          </div>
          <div class="set-row">
            <span class="lb">{tr('Fade-out curve')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('How the ending track fades away')}</span>
            </span>
            <Dropdown bind:value={settings.crossfade_curve_out} options={XF_CURVES} onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb">{tr('Fade-in curve')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('How the next track comes in')}</span>
            </span>
            <Dropdown bind:value={settings.crossfade_curve_in} options={XF_CURVES} onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb">{tr('Fade-out length')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{Math.round(settings.crossfade_out_len)}% of the crossfade, at the end</span>
            </span>
            <div style="width:200px">
              <Slider
                value={settings.crossfade_out_len - 10}
                max={90}
                live={true}
                showTip={true}
                format={(v) => `${Math.round(v + 10)}%`}
                oninput={(v) => { settings.crossfade_out_len = Math.round(v + 10); }}
                onchange={(v) => { settings.crossfade_out_len = Math.round(v + 10); api.setSettings(settings); }}
              />
            </div>
          </div>
          <div class="set-row">
            <span class="lb">{tr('Fade-in length')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{Math.round(settings.crossfade_in_len)}% of the crossfade, from the start</span>
            </span>
            <div style="width:200px">
              <Slider
                value={settings.crossfade_in_len - 10}
                max={90}
                live={true}
                showTip={true}
                format={(v) => `${Math.round(v + 10)}%`}
                oninput={(v) => { settings.crossfade_in_len = Math.round(v + 10); }}
                onchange={(v) => { settings.crossfade_in_len = Math.round(v + 10); api.setSettings(settings); }}
              />
            </div>
          </div>
          <div class="set-row">
            <span class="lb">{tr('Filter sweep')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('DJ-style EQ movement during the blend')}</span>
            </span>
            <Dropdown bind:value={settings.crossfade_filter} options={[
              { value: 'off', label: tr('Off') },
              { value: 'lowpass', label: tr('Low-pass out') },
              { value: 'highpass', label: tr('High-pass in') },
              { value: 'sweep', label: tr('Bass swap (both)') },
            ]} onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb">{tr('Smart start')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Skip silence at the end and start of tracks')}</span>
            </span>
            <Toggle bind:checked={settings.crossfade_smart} label={tr('Smart start')} onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb">{tr('Also on skip')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Same fade when you skip or pick another song')}</span>
            </span>
            <Toggle
              bind:checked={settings.crossfade_manual}
              label={tr('Also on skip')}
              disabled={!settings.crossfade}
              onchange={() => api.setSettings(settings)}
            />
          </div>
          <div class="set-row">
            <span class="lb">{tr('Album continuity')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('No crossfade between tracks on the same album')}</span>
            </span>
            <Toggle bind:checked={settings.album_continuity} label={tr('Album continuity')} onchange={() => api.setSettings(settings)} />
          </div>
          <div class="set-row">
            <span class="lb">{tr('BPM matching')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Detect tempo and match the next track to the current one')}</span>
            </span>
            <Toggle bind:checked={settings.bpm_match} label={tr('BPM matching')} onchange={() => api.setSettings(settings)} />
          </div>
          {#if settings.bpm_match}
            <div class="set-row">
              <span class="lb">{tr('Max tempo change')}
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">±{settings.bpm_max_change.toFixed(1)}% - larger gaps blend without matching</span>
              </span>
              <div style="width:200px">
                <Slider
                  value={settings.bpm_max_change}
                  max={16}
                  live={true}
                  showTip={true}
                  format={(v) => `±${Math.max(0.5, v).toFixed(1)}%`}
                  oninput={(v) => { settings.bpm_max_change = Math.max(0.5, Math.round(v * 2) / 2); }}
                  onchange={(v) => { settings.bpm_max_change = Math.max(0.5, Math.round(v * 2) / 2); api.setSettings(settings); }}
                />
              </div>
            </div>
            <div class="set-row">
              <span class="lb">{tr('Beat-aligned start')}
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Start the blend on a beat so kicks line up')}</span>
              </span>
              <Toggle bind:checked={settings.bpm_beat_align} label={tr('Beat-aligned start')} onchange={() => api.setSettings(settings)} />
            </div>
            <div class="set-row">
              <span class="lb">{tr('Keep pitch')}
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Time-stretch instead of speeding up, so vocals don\'t change key')}</span>
              </span>
              <Toggle bind:checked={settings.bpm_keylock} label={tr('Keep pitch')} onchange={() => api.setSettings(settings)} />
            </div>
            <div class="set-row">
              <span class="lb">{tr('Tempo return')}
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr("Glide back to the track's own tempo over {s}s", { s: settings.bpm_return_seconds.toFixed(0) })}</span>
              </span>
              <div style="width:200px">
                <Slider
                  value={settings.bpm_return_seconds}
                  max={30}
                  live={true}
                  showTip={true}
                  format={(v) => `${Math.max(1, Math.round(v))}s`}
                  oninput={(v) => { settings.bpm_return_seconds = Math.max(1, Math.round(v)); }}
                  onchange={(v) => { settings.bpm_return_seconds = Math.max(1, Math.round(v)); api.setSettings(settings); }}
                />
              </div>
            </div>
          {/if}
          <div class="set-row">
            <span class="lb">{tr('Crossfade defaults')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Restore every crossfade option above')}</span>
            </span>
            <button class="btn ghost" type="button" onclick={resetCrossfade}>{tr('Reset')}</button>
          </div>
          {/if}
          <div class="set-row">
            <span class="lb">{tr('Auto-DJ')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{player.shuffle ? tr('Reorder upcoming songs by tempo so transitions flow (paused while shuffle is on). Works best with BPM matching') : tr('Reorder upcoming songs by tempo so transitions flow. Works best with BPM matching')}</span>
            </span>
            <Toggle bind:checked={settings.auto_dj} label={tr('Auto-DJ')} onchange={() => { api.setSettings(settings); void refreshAnalysisCount(); }} />
          </div>
          {#if settings.auto_dj}
            <div class="set-row">
              <span class="lb">Harmonic mixing
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Also detect each song's key and prefer ones that blend in key (Camelot wheel)</span>
              </span>
              <Toggle checked={!!settings.harmonic_mix} label="Harmonic mixing" onchange={(v) => { settings.harmonic_mix = v; api.setSettings(settings); }} />
            </div>
          {/if}
          {#if settings.normalize || settings.bpm_match || settings.auto_dj}
            <div class="set-row">
              <span class="lb">{tr('Track analysis')}
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{analysisCount === 1 ? tr('Loudness and tempo saved for 1 track') : tr('Loudness and tempo saved for {n} tracks', { n: analysisCount })}</span>
              </span>
              <button class="btn ghost" type="button" onclick={clearAnalysis} disabled={analysisCount === 0}>{tr('Clear')}</button>
            </div>
          {/if}
            </div>
          </section>
          <section class="set-sec" id="set-sound">
            <div class="set-h"><h2>{tr('Sound')}</h2><p>{tr('Loudness and equalizer')}</p></div>
            <div class="set-card">
          {#if !isPhone}
            <div class="set-row">
              <span class="lb">Output device
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{outputs.current ? `Playing through ${outputs.current}` : `System default${outputs.default ? ` (${outputs.default})` : ''}`}</span>
              </span>
              <Dropdown value={settings.output_device ?? ''} options={[
                { value: '', label: 'System default' },
                ...[...new Set([...outputs.devices, ...(settings.output_device ? [settings.output_device] : [])])].map((d) => ({ value: d, label: outputs.devices.includes(d) ? d : `${d} (not connected)` })),
              ]} onchange={(v) => { settings.output_device = v || null; api.setSettings(settings); setTimeout(refreshOutputs, 600); }} />
            </div>
            <div class="set-row">
              <span class="lb">Switch automatically
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Move to headphones or Bluetooth when they connect, and back when they come back</span>
              </span>
              <Toggle checked={!!settings.output_auto_switch} label="Switch output automatically" onchange={(v) => { settings.output_auto_switch = v; api.setSettings(settings); }} />
            </div>
          {/if}
          <div class="set-row">
            <span class="lb">{tr('Volume normalization')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Measure each track and play them all at about the same loudness')}</span>
            </span>
            <Toggle bind:checked={settings.normalize} label={tr('Volume normalization')} onchange={() => { api.setSettings(settings); void refreshAnalysisCount(); }} />
          </div>
          {#if settings.normalize}
            <div class="set-row">
              <span class="lb">{tr('Target loudness')}
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{settings.normalize_target} LUFS - lower is quieter with more headroom</span>
              </span>
              <div style="width:200px">
                <Slider
                  value={settings.normalize_target + 23}
                  max={15}
                  live={true}
                  showTip={true}
                  format={(v) => `${Math.round(v - 23)} LUFS`}
                  oninput={(v) => { settings.normalize_target = Math.round(v - 23); }}
                  onchange={(v) => { settings.normalize_target = Math.round(v - 23); api.setSettings(settings); }}
                />
              </div>
            </div>
          {/if}
          <div class="set-row">
            <span class="lb">Speed & effects
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Nightcore, slowed + reverb, or your own mix</span>
            </span>
            <Dropdown value={fxPreset} options={[
              { value: 'normal', label: 'Normal' },
              { value: 'nightcore', label: 'Nightcore' },
              { value: 'slowed', label: 'Slowed + reverb' },
              { value: 'custom', label: 'Custom' },
            ]} onchange={(v) => applyFxPreset(v)} />
          </div>
          {#if fxPreset !== 'normal'}
            <div class="set-row">
              <span class="lb">Speed
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{fmtSpeed(settings.speed ?? 1)}</span>
              </span>
              <div style="width:200px">
                <Slider
                  value={Math.round(((settings.speed ?? 1) - 0.5) * 100)}
                  max={150}
                  live={true}
                  showTip={true}
                  step={5}
                  format={(v) => fmtSpeed(0.5 + Math.round(v) / 100)}
                  oninput={(v) => { settings.speed = 0.5 + Math.round(v) / 100; api.setSettings(settings); }}
                  onchange={(v) => { settings.speed = 0.5 + Math.round(v) / 100; api.setSettings(settings); }}
                />
              </div>
            </div>
            <div class="set-row">
              <span class="lb">Keep pitch
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Off: pitch follows the speed, like a record (nightcore / slowed)</span>
              </span>
              <Toggle checked={!!settings.speed_keep_pitch} label="Keep pitch" onchange={(v) => { settings.speed_keep_pitch = v; api.setSettings(settings); }} />
            </div>
            <div class="set-row">
              <span class="lb">Reverb
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{Math.round((settings.reverb ?? 0) * 100)}%</span>
              </span>
              <div style="width:200px">
                <Slider
                  value={Math.round((settings.reverb ?? 0) * 100)}
                  max={100}
                  live={true}
                  showTip={true}
                  format={(v) => `${Math.round(v)}%`}
                  oninput={(v) => { settings.reverb = Math.round(v) / 100; api.setSettings(settings); }}
                  onchange={(v) => { settings.reverb = Math.round(v) / 100; api.setSettings(settings); }}
                />
              </div>
            </div>
          {/if}
          <div class="set-row set-eq">
            <div class="set-eq-head">
              <span class="lb"><Hi icon={SlidersHorizontalIcon} size={20} /> {tr('Equalizer')}
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Five bands, −12 to +12 dB')}</span>
              </span>
              <Toggle bind:checked={settings.eq_enabled} label={tr('Equalizer')} onchange={() => api.setSettings(settings)} />
            </div>
            <div class="eq-presets" role="listbox" aria-label={tr('Equalizer presets')}>
              {#each EQ_PRESETS as p}
                <button
                  class="eq-preset"
                  class:on={eqPresetId === p.id}
                  type="button"
                  role="option"
                  aria-selected={eqPresetId === p.id}
                  onclick={() => applyEqPreset(p.bands)}
                >{p.label}</button>
              {/each}
            </div>
            <div class="eq-bands" class:off={!settings.eq_enabled}>
              {#each settings.eq_bands as gain, i}
                <div class="eq-band">
                  <span class="eq-gain">{gain > 0 ? '+' : ''}{gain.toFixed(1)}</span>
                  <div class="eq-sl">
                    <Slider
                      value={gain + 12}
                      max={24}
                      step={0.5}
                      live
                      showTip
                      format={(v) => `${(v - 12) > 0 ? '+' : ''}${(v - 12).toFixed(1)} dB`}
                      oninput={(v) => {
                        const bands = [...settings.eq_bands] as EqBands;
                        bands[i] = Math.round((v - 12) * 10) / 10;
                        settings.eq_bands = bands;
                      }}
                      onchange={(v) => setEqBand(i, v - 12)}
                    />
                  </div>
                  <span class="eq-hz">{EQ_LABELS[i]}</span>
                </div>
              {/each}
            </div>
          </div>
            </div>
          </section>
          <section class="set-sec" id="set-lyrics">
            <div class="set-h"><h2>{tr('Lyrics & visuals')}</h2><p>{tr('Synced lyrics and the fullscreen player')}</p></div>
            <div class="set-card">
              <div class="set-row">
                <span class="lb">{tr('Letter-by-letter fill')}
                  <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Shows where in the line the singer is')}</span>
                </span>
                <Toggle bind:checked={settings.lyrics_karaoke} label={tr('Letter-by-letter fill')} onchange={() => api.setSettings(settings)} />
              </div>
              <div class="set-row">
                <span class="lb">Synced lyrics from LRCLIB
                  <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">When YouTube Music has no timed lyrics, look the song up on lrclib.net</span>
                </span>
                <Toggle checked={settings.lyrics_lrclib !== false} label="Synced lyrics from LRCLIB" onchange={(v) => { settings.lyrics_lrclib = v; api.setSettings(settings); }} />
              </div>
              <div class="set-row">
                <span class="lb">Glow
                  <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Soft light on the letters being sung</span>
                <span class="lb">{tr('Glow')}
                  <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Soft light on the letters being sung')}</span>
                </span>
                <Toggle bind:checked={settings.lyrics_glow} label={tr('Glow')} onchange={() => api.setSettings(settings)} />
              </div>
              <div class="set-row">
                <span class="lb">{tr('Blur distant lines')}
                  <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Fullscreen only: lines fade out of focus the further they are')}</span>
                </span>
                <Toggle bind:checked={settings.lyrics_blur} label={tr('Blur distant lines')} onchange={() => api.setSettings(settings)} />
              </div>
              <div class="set-row">
                <span class="lb">{tr('Instrumental dots')}
                  <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Three dots fill up during long gaps between lines')}</span>
                </span>
                <Toggle bind:checked={settings.lyrics_dots} label={tr('Instrumental dots')} onchange={() => api.setSettings(settings)} />
              </div>
              <div class="set-row">
                <span class="lb">Visualizer
                  <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Fullscreen only: the music's spectrum, live, in the cover's colour</span>
                </span>
                <Dropdown value={settings.visualizer ?? 'off'} options={[
                  { value: 'off', label: 'Off' },
                  { value: 'bars', label: 'Bars' },
                  { value: 'wave', label: 'Wave' },
                ]} onchange={(v) => { settings.visualizer = v as 'off' | 'bars' | 'wave'; api.setSettings(settings); }} />
              </div>
              <div class="set-row">
                <span class="lb">Text size
                <span class="lb">{tr('Text size')}
                  <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{Math.round(settings.lyrics_size)}%</span>
                </span>
                <div style="width:200px">
                  <Slider
                    value={settings.lyrics_size - 70}
                    max={80}
                    live={true}
                    showTip={true}
                    format={(v) => `${Math.round(v) + 70}%`}
                    oninput={(v) => { settings.lyrics_size = Math.round(v) + 70; }}
                    onchange={(v) => { settings.lyrics_size = Math.round(v) + 70; api.setSettings(settings); }}
                  />
                </div>
              </div>
            </div>
          </section>
          <section class="set-sec" id="set-storage">
            <div class="set-h"><h2>{tr('Offline & storage')}</h2><p>{tr('Downloads and cache on this device')}</p></div>
            <div class="set-card">
          <div class="set-row">
            <span class="lb"><Hi icon={WifiOff01Icon} size={20} /> {tr('Offline mode')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{off.netDown && !off.mode ? tr('Play only downloaded songs and never touch the network (no connection detected right now)') : tr('Play only downloaded songs and never touch the network')}</span>
            </span>
            <Toggle checked={off.mode} label={tr('Offline mode')} onchange={(v) => api.setOfflineMode(v)} />
          </div>
          <div class="set-row">
            <span class="lb"><Hi icon={Download04Icon} size={20} /> {tr('Downloads')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('{n} collections · {size} on this device', { n: off.status?.sources.length ?? 0, size: fmtBytes(off.status?.bytes ?? 0) })}</span>
            </span>
            <span style="display:flex;gap:8px">
              <button class="btn ghost sm" type="button" onclick={() => go({ id: 'downloads' })}>{tr('Manage')}</button>
              {#if off.status?.sources.length}
                <button class="btn ghost sm" type="button" class:on={confirmRm === 'all'} onclick={() => { if (confirmRm === 'all') { api.offlineRemoveAll(); confirmRm = null; } else { confirmRm = 'all'; setTimeout(() => { if (confirmRm === 'all') confirmRm = null; }, 3000); } }}>{confirmRm === 'all' ? tr('Really remove all?') : tr('Remove all')}</button>
              {/if}
            </span>
          </div>
          <div class="set-row set-cache">
            <span class="lb"><Hi icon={Bookmark02Icon} size={20} /> {tr('Offline cache')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{fmtCache(cacheUsed)} of {fmtCache(cacheMax)}</span>
            </span>
            <div class="cache-ctl">
              <Dropdown
                value={cacheMode === 'custom' ? 'custom' : cacheSelect}
                options={CACHE_PRESETS}
                onchange={(v) => { void applyCachePreset(v); }}
              />
              {#if cacheMode === 'custom' || cacheSelect === 'custom'}
                <div class="cache-custom">
                  <input
                    type="number"
                    min="0"
                    step="any"
                    inputmode="decimal"
                    aria-label={tr('Cache size')}
                    bind:value={cacheCustomAmt}
                    onchange={() => { void applyCacheCustom(); }}
                    onkeydown={(e) => { if (e.key === 'Enter') void applyCacheCustom(); }}
                  />
                  <Dropdown
                    value={cacheCustomUnit}
                    options={[
                      { value: 'MB', label: tr('MB') },
                      { value: 'GB', label: tr('GB') },
                      { value: 'TB', label: tr('TB') },
                      { value: 'PB', label: tr('PB') },
                      { value: 'EB', label: tr('EB') },
                    ]}
                    onchange={(v) => { cacheCustomUnit = v as CacheUnit; void applyCacheCustom(); }}
                  />
                </div>
              {/if}
            </div>
          </div>
            </div>
          </section>
          <section class="set-sec" id="set-connect">
            <div class="set-h"><h2>{tr('Connections & privacy')}</h2><p>{tr('Discord, scrobbling and your history')}</p></div>
            <div class="set-card">
          <div class="set-row">
            <span class="lb"><Hi icon={DiscordIcon} size={20} /> {tr('Discord presence')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Show the track and cover on your Discord profile')}</span>
            </span>
            <Toggle bind:checked={settings.discord_presence} label={tr('Discord presence')} onchange={() => api.setSettings(settings)} />
          </div>
          <Scrobbling onChange={() => scrobbler.reload()} />
          <div class="set-row">
            <span class="lb">{tr('Listening stats')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Keep a private history on this device for the Stats page')}</span>
            </span>
            <Toggle bind:checked={settings.listening_stats} label={tr('Listening stats')} onchange={() => api.setSettings(settings)} />
          </div>
          {#if !isPhone}
            <PhoneRemote />
            <ListenTogether />
          {/if}
            </div>
          </section>
          <section class="set-sec" id="set-system">
            <div class="set-h"><h2>{tr('Shortcuts & system')}</h2><p>{tr('Keyboard and tray')}</p></div>
            <div class="set-card">
          <Shortcuts
            shortcuts={settings.shortcuts}
            globalHotkeys={settings.global_hotkeys}
            desktop={!/Android|iPhone|iPad/i.test(navigator.userAgent)}
            mac={isMac}
            onChange={(sc, gh) => { void saveShortcuts(sc, gh); }}
          />
          {#if !isPhone}
            <div class="set-row">
              <span class="lb">{tr('Tray icon')}
                <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Play/pause, next, previous and like from the {isMac ? 'menu bar' : 'system tray'}</span>
              </span>
              <Toggle bind:checked={settings.tray_icon} label={tr('Tray icon')} onchange={() => { if (!settings.tray_icon) settings.close_to_tray = false; api.setSettings(settings); }} />
            </div>
            {#if settings.tray_icon}
              <div class="set-row">
                <span class="lb">{tr('Close to tray')}
                  <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Closing the window keeps the music playing; quit from the tray menu')}</span>
                </span>
                <Toggle bind:checked={settings.close_to_tray} label={tr('Close to tray')} onchange={() => api.setSettings(settings)} />
              </div>
            {/if}
          {/if}
            </div>
          </section>
          <section class="set-sec" id="set-about">
            <div class="set-h"><h2>{tr('About')}</h2><p>{tr('Updates, diagnostics and account')}</p></div>
            <div class="set-card">
          <div class="set-row">
            <span class="lb">{tr('Updates')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">Tarn {appVersion}</span>
            </span>
            <button class="upd-check" type="button" onclick={() => (updateCheck += 1)}>{tr('Check for updates')}</button>
          </div>
          <div class="set-row">
            <span class="lb">{tr('Diagnostics')}
              <span class="t-meta" style="display:block;font-weight:450;margin-top:2px">{tr('Auth, library, and recent logs - no cookie values.')}</span>
            </span>
            <button class="btn ghost" type="button" onclick={copyDiagnostics}>{tr('Copy diagnostics')}</button>
          </div>
          {#if diagText !== null}
            <div style="position:fixed;inset:0;z-index:9999;background:rgba(0,0,0,.85);display:flex;flex-direction:column;gap:8px;padding:calc(env(safe-area-inset-top) + 12px) 12px calc(env(safe-area-inset-bottom) + 12px)">
              <textarea readonly style="flex:1;width:100%;box-sizing:border-box;font:11px/1.35 monospace;background:#121212;color:#eee;border:1px solid #444;border-radius:8px;padding:8px" onfocus={(e) => e.currentTarget.select()}>{diagText}</textarea>
              <button class="btn ghost" type="button" onclick={() => (diagText = null)}>{tr('Close')}</button>
            </div>
          {/if}
          {#if auth.signed_in}
            <div class="set-row">
              <span class="lb"><Hi icon={Logout03Icon} size={20} /> {auth.account?.name || 'Account'}</span>
              <button class="btn ghost" type="button" onclick={async () => { await api.signOut(); auth = { signed_in: false, mock: false, account: null, store: null }; library = null; clearHomeCaches(); home = null; explore = null; if (view.id === 'home') loadHome(); if (view.id === 'explore') loadExplore(); }}>{tr('Sign out')}</button>
            </div>
          {/if}
            </div>
          </section>
        </div>
      {/if}
    </div>
  </main>

  {#if sidePanelOn && !expanded && sidePanelView}
    <aside class="panel">
      <div class="panel-h">
        <h2>{{ queue: 'Up next', lyrics: 'Lyrics', related: 'Related' }[sidePanelView]}</h2>
        <IconBtn icon={Cancel01Icon} size={24} btnSize="lg" onclick={closePanels} title={tr('Close')} />
      </div>
      <div class="ptabs">
        <button class="ptab {sidePanelView === 'queue' ? 'on' : ''}" type="button" onclick={() => openPanel('queue')}>{tr('Queue')}</button>
        <button class="ptab {sidePanelView === 'lyrics' ? 'on' : ''}" type="button" disabled={lyricsReady && !lyricsOk} onclick={() => openPanel('lyrics')}>{tr('Lyrics')}</button>
        <button class="ptab {sidePanelView === 'related' ? 'on' : ''}" type="button" onclick={() => openPanel('related')}>{tr('Related')}</button>
      </div>
      <div class="panel-b">
        {#if sidePanelView === 'queue'}
          {#if player.queue.length}
            <div class="q-bar">
              <span>{tr('{n} up next', { n: Math.max(0, player.queue.length - player.index - 1) })}</span>
              <button class="q-clear" type="button" onclick={saveQueueAsPlaylist}>{tr('Save as playlist')}</button>
              {#if player.queue.length - player.index > 1}
                <button class="q-clear" type="button" onclick={() => api.queueClear()}>{tr('Clear')}</button>
              {/if}
            </div>
          {/if}
          <QueueList
            tracks={player.queue}
            index={player.index}
            onPlay={(i) => playTracks(player.queue, i)}
            onRemove={(i) => api.queueRemove(i)}
            onMove={(f, t) => api.queueMove(f, t)}
            onContext={(e, t) => contextTrack(e, t)}
            onHover={(t) => t.id && api.prefetch(t.id)}
          />
        {:else if sidePanelView === 'lyrics'}
          <div class="lyrics" class:no-glow={!settings.lyrics_glow} style:--ly-scale={settings.lyrics_size / 100}>
            {#if lyrics && lyrics.lines.length}
              {#if !lyrics.synced}
                <div class="ly-note">{tr('Lyrics for this aren\'t synced yet')}</div>
              {/if}
              {#each lyrics.lines as ln, i}
                {@render gapDots(i)}
                <button
                  type="button"
                  class="ly {i === activeLyric && gapNow < 0 ? 'on' : ''}"
                  class:static={!lyrics.synced || ln.start_ms == null}
                  data-i={i}
                  onclick={() => lyrics?.synced && ln.start_ms != null && api.seek(ln.start_ms / 1000)}
                >{#if i === activeLyric && gapNow < 0 && settings.lyrics_karaoke}{@render karaoke()}{:else}{ln.text}{/if}</button>
              {/each}
              {#if lyrics.source}<div class="ly-src">{lyrics.source}</div>{/if}
            {:else}
              <div class="empty"></div>
            {/if}
          </div>
        {:else if sidePanelView === 'related' && related}
          {#if related.tracks.length}
            {#each related.tracks as t, i (t.id || i)}
              <button class="q-item" type="button" onclick={() => playTracks(related!.tracks, i)} oncontextmenu={(e) => contextTrack(e, t)}>
                <Cover src={t.cover} size={40} alt="" />
                <div class="tx"><div class="tn">{t.title}</div><div class="ta">{artistsLine(t.artists)}</div></div>
              </button>
            {/each}
          {/if}
          {#if related.artists.length}
            <div class="shelf-title" style="font-size:16px;margin:16px 8px 8px">{tr('Artists')}</div>
            <div class="row">
              {#each related.artists as a (a.id)}
                <button class="card artist" type="button" onclick={() => go({ id: 'artist', artistId: a.id })} oncontextmenu={(e) => ctxColl(e, { kind: 'artist', id: a.id, title: a.name, cover: a.cover })}>
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

  {#if expanded}
    <div
      class="np"
      class:has-lyrics={npLyricsOn || (npLyricsMode && lyricsReady)}
      class:has-queue={npQueueOn}
      onwheel={nudgeVolume}
    >
      <button
        class="ibtn lg np-close"
        type="button"
        onclick={() => (expanded = false)}
        title={tr('Close')}
        onpointerenter={() => { npHoverChrome = true; armNpHide(); }}
        onpointerleave={() => { npHoverChrome = false; armNpHide(); }}
      >
        <Hi icon={Cancel01Icon} size={26} />
      </button>
      {#if player.track}
        <!-- Outside the keyed stage: stays put on a skip and only swaps its image,
             so the blurred backdrop never flashes empty/half-painted. -->
        <Cover src={nowCover} videoId={player.track?.id} width={320} height={200} class="np-bg" eager alt="" />
        <div class="np-shade"></div>
        {#if settings.visualizer === 'bars' || settings.visualizer === 'wave'}
          <Visualizer mode={settings.visualizer} playing={player.playing} />
        {/if}
        {#key player.track.id}
          <div
            class="np-stage"
            class:slide-next={npSlide === 'next'}
            class:slide-prev={npSlide === 'prev'}
          >
                        <div class="np-in">
              <div class="np-main" role="presentation" oncontextmenu={(e) => { if (player.track) contextTrack(e, player.track); }}>
                <div class="np-art">
                  <Cover src={nowCover} videoId={player.track?.id} size={560} eager alt="" />
                  {#if volHud != null}
                    <div class="vol-hud lg" aria-live="polite">{volHud}%</div>
                  {/if}
                </div>
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
              {#if npLyricsOn && lyrics}
                <div
                  class="np-lyrics"
                  class:no-glow={!settings.lyrics_glow}
                  class:no-blur={!settings.lyrics_blur}
                  style:--ly-scale={settings.lyrics_size / 100}
                  aria-label={tr('Lyrics')}
                  bind:this={npLyricsEl}
                  onwheel={(e) => e.stopPropagation()}
                >
                  {#each lyrics.lines as ln, i}
                    {@render gapDots(i)}
                    <button
                      type="button"
                      class="ly"
                      class:on={i === activeLyric && gapNow < 0}
                      class:past={activeLyric >= 0 && (i < activeLyric || (i === activeLyric && gapNow >= 0))}
                      class:static={ln.start_ms == null}
                      data-i={i}
                      style:--d={activeLyric < 0 ? 3 : Math.min(4, Math.abs(i - activeLyric))}
                      onclick={() => ln.start_ms != null && api.seek(ln.start_ms / 1000)}
                    >{#if i === activeLyric && gapNow < 0 && settings.lyrics_karaoke}{@render karaoke()}{:else}{ln.text || ' '}{/if}</button>
                  {/each}
                </div>
              {:else if npLyricsMode && lyricsReady && !npLyricsOn}
                <div class="np-lyrics np-lyrics-empty" aria-label={tr('Lyrics')}>
                  <div class="ly-note">{tr('No synced lyrics for this track')}</div>
                </div>
              {/if}
            </div>
          </div>
        {/key}
      {/if}
      {#if npQueueOn}
        <aside
          class="np-queue"
          class:dim={!npShow}
          aria-label={tr('Up next')}
          onwheel={(e) => e.stopPropagation()}
          onpointerenter={() => { npHoverChrome = true; armNpHide(); }}
          onpointerleave={() => { npHoverChrome = false; armNpHide(); }}
        >
          <div class="np-queue-h">{tr('Up next')}</div>
          <div class="np-queue-list">
            <QueueList
              variant="np"
              tracks={player.queue}
              index={player.index}
              onPlay={(i) => playTracks(player.queue, i)}
              onRemove={(i) => api.queueRemove(i)}
              onMove={(f, t) => api.queueMove(f, t)}
              onContext={(e, t) => contextTrack(e, t)}
              onHover={(t) => t.id && api.prefetch(t.id)}
            />
            {#if player.queue.length - player.index <= 1}
              <div class="np-queue-empty">{tr('Nothing else in the queue')}</div>
            {/if}
          </div>
        </aside>
      {/if}
    </div>
  {/if}

  <footer
    class="player"
    onpointerenter={() => { if (expanded) { npHoverChrome = true; armNpHide(); } }}
    onpointerleave={() => { if (expanded) { npHoverChrome = false; armNpHide(); } }}
  >
    <div class="player-in">
      <div class="now" role="presentation" onwheel={nudgeVolume} oncontextmenu={(e) => { if (player.track) contextTrack(e, player.track); }}>
        {#if player.track}
          <button class="cov" type="button" onclick={() => (expanded = !expanded)} title={tr('Now playing')}>
            <Cover src={nowCover} videoId={player.track?.id} size={64} alt="" />
            {#if volHud != null}
              <div class="vol-hud" aria-live="polite">{volHud}%</div>
            {/if}
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
            <IconBtn
              icon={FavouriteIcon}
              altIcon={FavouriteFilledIcon}
              showAlt={player.liked}
              size={22}
              on={player.liked}
              onclick={toggleLike}
              title={tr('Like')}
            />
            <IconBtn icon={ThumbsDownIcon} size={22} on={player.disliked} danger onclick={toggleDislike} title={tr('Dislike')} />
          </div>
        {/if}
      </div>
      <div class="ctrls">
        <div class="row-c">
          <IconBtn icon={ShuffleIcon} size={22} on={player.shuffle} onclick={toggleShuffle} title={tr('Shuffle')} btnSize="lg" />
          <IconBtn icon={PreviousIcon} size={26} btnSize="lg" onclick={() => api.prev()} title={tr('Previous')} />
          <IconBtn
            icon={PlayIcon}
            altIcon={PauseIcon}
            showAlt={player.playing && !player.buffering}
            size={28}
            btnSize="xl"
            spinning={player.buffering || (player.loading && !player.playing)}
            onclick={() => api.toggle()}
            title={player.buffering ? 'Buffering' : player.playing ? tr('Pause') : tr('Play')}
            disabled={!player.track && !player.loading}
          />
          <IconBtn icon={NextIcon} size={26} btnSize="lg" onclick={() => api.next()} title={tr('Next')} />
          <IconBtn
            icon={RepeatIcon}
            altIcon={RepeatOne01Icon}
            showAlt={player.repeat === 'one'}
            size={22}
            on={player.repeat !== 'off'}
            btnSize="lg"
            onclick={cycleRepeatUi}
            title={tr('Repeat')}
          />
        </div>
        <div class="seek">
          <span class="t">{fmtTime(scrub ?? player.position)}</span>
          <Slider
            value={player.position}
            max={player.duration || 0}
            buffered={player.buffered}
            {wave}
            disabled={!player.track || !(player.duration > 0)}
            showTip
            format={fmtTime}
            onpreview={previewSeek}
            onchange={commitSeek}
          />
          <span class="t end">{fmtTime(player.duration)}</span>
          {#if (settings.speed ?? 1) !== 1 || (settings.reverb ?? 0) > 0}
            <button class="speed-chip" title="Back to normal speed" onclick={() => applyFxPreset('normal')}>{fxPreset === 'slowed' ? 'slowed' : fxPreset === 'nightcore' ? 'nightcore' : fmtSpeed(settings.speed ?? 1)}</button>
          {/if}
        </div>
      </div>
      <div class="right-c">
        {#if player.error}<span class="err">{friendlyError(player.error, "Can't play this track")}</span>{/if}
        {#if player.loading && !player.playing}<div class="spin"></div>{/if}
        <IconBtn
          icon={MicVocalIcon}
          size={22}
          on={lyricsOn}
          disabled={!player.track || (lyricsReady && !lyricsOk)}
          onclick={() => openPanel('lyrics')}
          title={lyricsReady && !lyricsOk ? tr('No lyrics') : tr('Lyrics')}
          btnSize="lg"
        />
        <IconBtn icon={Queue02Icon} size={22} on={queueOn} onclick={() => openPanel('queue')} title={tr('Queue')} btnSize="lg" />
        <IconBtn icon={Radio02Icon} size={22} onclick={() => startRadio()} title={tr('Radio')} btnSize="lg" />
        {#if !/Android|iPhone|iPad/i.test(navigator.userAgent)}
          <IconBtn icon={MiniPlayerIcon} size={22} onclick={() => openMini()} title={tr('Mini player')} btnSize="lg" />
        {/if}
        <SleepTimer
          trackId={player.track?.id ?? null}
          position={player.position}
          duration={player.duration}
          onStop={() => { api.pause().catch(() => {}); toast(tr('Sleep timer: paused'), 'info', 2600); }}
        />
        <div class="vol">
          <Hi icon={player.volume < 0.01 ? VolumeMute01Icon : VolumeHighIcon} size={20} />
          <Slider
            value={player.volume}
            max={1}
            step={0.01}
            live
            wheel
            showTip
            format={(v) => `${Math.round(v * 100)}%`}
            oninput={(v) => setVolumePrecise(v)}
          />
        </div>
      </div>
    </div>
  </footer>
</div>

<Toast />
{#if importOpen}
  <ImportPlaylist
    onClose={() => (importOpen = false)}
    onDone={(n, a, t) => { toast(tr('Imported {a} of {t} songs into {name}', { a, t, name: n }), 'info', 3200); loadLibrary(); }}
  />
{/if}
<UpdatePrompt bind:request={updateCheck} />

{#if ctx}
  <ContextMenu x={ctx.x} y={ctx.y} entries={buildMenu(ctx)} onClose={() => (ctx = null)} />
{/if}
