<script lang="ts">
  import type { Track } from '../types';
  import { fmtTime, artistsLine } from '../format';
  import Cover from './Cover.svelte';
  import Hi from './Hi.svelte';
  import { MoreHorizontalIcon, PlayIcon } from '../icons';

  let {
    tracks,
    currentId = null,
    playing = false,
    showAlbum = true,
    showCover = true,
    numbered = true,
    onPlay,
    onArtist,
    onAlbum,
    onContext,
    onMore,
    fallbackCover = null,
  }: {
    tracks: Track[];
    currentId?: string | null;
    playing?: boolean;
    showAlbum?: boolean;
    showCover?: boolean;
    numbered?: boolean;
    onPlay: (t: Track, i: number) => void;
    onArtist?: (id: string, name: string) => void;
    onAlbum?: (id: string) => void;
    onContext: (e: MouseEvent, t: Track, i: number) => void;
    onMore?: (e: MouseEvent, t: Track, i: number) => void;
    fallbackCover?: string | null;
  } = $props();
</script>

<table class="tbl">
  <thead>
    <tr>
      {#if numbered}<th style="width:36px">#</th>{/if}
      <th>Title</th>
      {#if showAlbum}<th class="al">Album</th>{/if}
      <th style="width:56px"></th>
      <th style="width:48px"></th>
    </tr>
  </thead>
  <tbody>
    {#each tracks as t, i}
      <tr
        class="trow {t.id === currentId ? 'on' : ''}"
        ondblclick={() => onPlay(t, i)}
        oncontextmenu={(e) => onContext(e, t, i)}
      >
        {#if numbered}
          <td class="idx">
            {#if t.id === currentId && playing}
              <span style="color:rgb(var(--accent-rgb))">●</span>
            {:else}
              {i + 1}
            {/if}
          </td>
        {/if}
        <td>
          <div style="display:flex;align-items:center;gap:12px;min-width:0">
            {#if showCover}
              <button class="cov" onclick={() => onPlay(t, i)} title="Play" type="button">
                <Cover src={t.cover || fallbackCover} size={40} alt="" />
              </button>
            {/if}
            <div style="min-width:0">
              <div class="tn clamp one">{t.title}</div>
              <div class="ta">
                {#each t.artists as a, k}
                  {#if k > 0}<span>, </span>{/if}
                  <button
                    class="linkish"
                    type="button"
                    onclick={() => onArtist?.(t.artist_ids[k] || '', a)}
                  >{a}</button>
                {/each}
              </div>
            </div>
          </div>
        </td>
        {#if showAlbum}
          <td class="al">
            {#if t.album}
              <button
                class="linkish"
                type="button"
                onclick={() => t.album_id && onAlbum?.(t.album_id)}
              >{t.album}</button>
            {/if}
          </td>
        {/if}
        <td class="dur">{t.duration ? fmtTime(t.duration) : ''}</td>
        <td class="acts">
          <button
            class="ibtn"
            style="width:36px;height:36px;min-width:36px;min-height:36px"
            type="button"
            onclick={(e) => onMore?.(e, t, i) ?? onContext(e, t, i)}
            title="More"
          >
            <Hi icon={MoreHorizontalIcon} size={18} />
          </button>
        </td>
      </tr>
    {/each}
  </tbody>
</table>
