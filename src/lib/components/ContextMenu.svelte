<script lang="ts">
  import Hi from './Hi.svelte';
  import {
    PlayIcon, PlayListAddIcon, Queue02Icon, Radio02Icon, FavouriteIcon, ThumbsDownIcon,
    Share01Icon, Link01Icon, Album02Icon, UserIcon, Add01Icon, Bookmark02Icon,
  } from '../icons';
  import type { PlaylistCard, Track } from '../types';

  let {
    x,
    y,
    track = null,
    signedIn = false,
    playlists = [],
    onPlay,
    onPlayNext,
    onQueue,
    onRadio,
    onLike,
    onDislike,
    onArtist,
    onAlbum,
    onShare,
    onAddTo,
    onSave,
    onClose,
  }: {
    x: number;
    y: number;
    track?: Track | null;
    signedIn?: boolean;
    playlists?: PlaylistCard[];
    onPlay?: () => void;
    onPlayNext?: () => void;
    onQueue?: () => void;
    onRadio?: () => void;
    onLike?: () => void;
    onDislike?: () => void;
    onArtist?: () => void;
    onAlbum?: () => void;
    onShare?: () => void;
    onAddTo?: (id: string) => void;
    onSave?: () => void;
    onClose: () => void;
  } = $props();

  let addOpen = $state(false);
  let el: HTMLDivElement | undefined = $state();

  $effect(() => {
    const on = (e: MouseEvent) => {
      if (el && !el.contains(e.target as Node)) onClose();
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('mousedown', on);
    window.addEventListener('keydown', esc);
    return () => {
      window.removeEventListener('mousedown', on);
      window.removeEventListener('keydown', esc);
    };
  });

  let left = $derived(Math.min(x, (typeof window !== 'undefined' ? window.innerWidth : 800) - 248));
  let top = $derived(Math.min(y, (typeof window !== 'undefined' ? window.innerHeight : 600) - 420));
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="ctx" bind:this={el} style="left:{Math.round(left)}px;top:{Math.round(top)}px" role="menu">
  {#if onPlay}
    <button type="button" onclick={() => { onPlay(); onClose(); }}><span class="ic"><Hi icon={PlayIcon} size={18} /></span> Play</button>
  {/if}
  <button type="button" onclick={() => { onPlayNext?.(); onClose(); }}><span class="ic"><Hi icon={PlayListAddIcon} size={18} /></span> Play next</button>
  <button type="button" onclick={() => { onQueue?.(); onClose(); }}><span class="ic"><Hi icon={Queue02Icon} size={18} /></span> Add to queue</button>
  <button type="button" onclick={() => { onRadio?.(); onClose(); }}><span class="ic"><Hi icon={Radio02Icon} size={18} /></span> Start radio</button>
  <div class="sep"></div>
  <button type="button" onclick={() => { onLike?.(); onClose(); }}><span class="ic"><Hi icon={FavouriteIcon} size={18} /></span> Like</button>
  <button type="button" onclick={() => { onDislike?.(); onClose(); }}><span class="ic"><Hi icon={ThumbsDownIcon} size={18} /></span> Dislike</button>
  {#if signedIn}
    <button type="button" onclick={() => (addOpen = !addOpen)}><span class="ic"><Hi icon={Add01Icon} size={18} /></span> Add to playlist</button>
    {#if addOpen}
      {#each playlists as p}
        <button type="button" style="padding-left:40px" onclick={() => { onAddTo?.(p.id); onClose(); }}>{p.name}</button>
      {/each}
      <button type="button" style="padding-left:40px" onclick={() => { onAddTo?.(''); onClose(); }}>New playlist</button>
    {/if}
    <button type="button" onclick={() => { onSave?.(); onClose(); }}><span class="ic"><Hi icon={Bookmark02Icon} size={18} /></span> Save to library</button>
  {/if}
  <div class="sep"></div>
  {#if track?.artist_ids?.[0] || track?.artists?.[0]}
    <button type="button" onclick={() => { onArtist?.(); onClose(); }}><span class="ic"><Hi icon={UserIcon} size={18} /></span> Go to artist</button>
  {/if}
  {#if track?.album_id || track?.album}
    <button type="button" onclick={() => { onAlbum?.(); onClose(); }}><span class="ic"><Hi icon={Album02Icon} size={18} /></span> Go to album</button>
  {/if}
  <button type="button" onclick={() => { onShare?.(); onClose(); }}><span class="ic"><Hi icon={Share01Icon} size={18} /></span> Share link</button>
</div>
