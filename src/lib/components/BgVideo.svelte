<script lang="ts">
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { api, type BgVideo, type VideoSeg } from '../api';

  let {
    trackId,
    isVideo = false,
    /** Playback position of the song, seconds. */
    position,
    playing,
    rate = 1,
    /** Darkening, percent 0..80. */
    dim = 35,
    /** Called with true while the video is actually showing. */
    onshow,
  }: {
    trackId: string | null;
    isVideo?: boolean;
    position: number;
    playing: boolean;
    rate?: number;
    dim?: number;
    onshow?: (on: boolean) => void;
  } = $props();

  let info = $state<BgVideo | null>(null);
  let el = $state<HTMLVideoElement | null>(null);
  let ready = $state(false);
  let failed = $state(false);
  /** Song time falls inside the part of the video that matches it. */
  let covered = $state(true);
  let reqFor = '';

  $effect(() => {
    const id = trackId;
    const v = isVideo;
    if (!id) {
      reqFor = '';
      info = null;
      return;
    }
    if (reqFor === id) return;
    reqFor = id;
    info = null;
    ready = false;
    failed = false;
    api
      .bgVideo(id, v)
      .then((r) => {
        if (reqFor === id) info = r;
      })
      .catch(() => {
        if (reqFor === id) info = null;
      });
  });

  const src = $derived(info ? convertFileSrc(info.file, 'tarnvideo') : null);
  const shown = $derived(!!info && ready && !failed && covered);

  $effect(() => {
    onshow?.(shown);
  });
  $effect(() => () => onshow?.(false));

  /** Song ms -> video ms; null when the video has nothing for that moment. */
  function toVideo(ms: number, segs: VideoSeg[]): number | null {
    if (!segs.length) return ms;
    for (const g of segs) {
      if (ms >= g.primary_ms && ms < g.primary_ms + g.duration_ms) {
        return g.counterpart_ms + (ms - g.primary_ms);
      }
    }
    return null;
  }

  // Follow the audio: small drift is absorbed by nudging the playback rate,
  // big jumps (seek, track start) by seeking the video.
  $effect(() => {
    const v = el;
    const i = info;
    const pos = position;
    const on = playing;
    const r = rate > 0 ? rate : 1;
    if (!v || !i || failed) return;
    const target = toVideo(pos * 1000, i.segments);
    const end = Number.isFinite(v.duration) ? v.duration : Infinity;
    if (target == null || target / 1000 >= end - 0.05) {
      covered = false;
      if (!v.paused) v.pause();
      return;
    }
    covered = true;
    if (v.readyState < 1) return;
    const t = target / 1000;
    const drift = t - v.currentTime;
    if (Math.abs(drift) > 0.6 || !on) {
      if (Math.abs(drift) > 0.05) v.currentTime = t;
      v.playbackRate = r;
    } else {
      v.playbackRate = r * (1 + Math.max(-0.08, Math.min(0.08, drift * 0.5)));
    }
    if (on && v.paused) v.play().catch(() => {});
    if (!on && !v.paused) v.pause();
  });
</script>

<div class="bgv" class:on={shown} style:--dim={Math.max(0, Math.min(80, dim)) / 100} aria-hidden="true">
  {#if src}
    {#key src}
      <video
        bind:this={el}
        {src}
        muted
        playsinline
        preload="auto"
        disablepictureinpicture
        onloadedmetadata={() => {
          if (el && info) {
            const t = toVideo(position * 1000, info.segments);
            if (t != null) el.currentTime = t / 1000;
          }
        }}
        oncanplay={() => (ready = true)}
        onerror={() => (failed = true)}
      ></video>
    {/key}
  {/if}
  <div class="bgv-shade"></div>
</div>

<style>
  .bgv {
    position: absolute;
    inset: 0;
    z-index: 0;
    pointer-events: none;
    opacity: 0;
    transition: opacity 0.7s ease;
    background: #000;
  }
  .bgv.on { opacity: 1; }
  video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  /* Darken evenly, a bit more at the top and bottom where the chrome sits. */
  .bgv-shade {
    position: absolute;
    inset: 0;
    background:
      linear-gradient(to bottom, rgba(0, 0, 0, 0.35) 0%, rgba(0, 0, 0, 0) 22%, rgba(0, 0, 0, 0) 70%, rgba(0, 0, 0, 0.45) 100%),
      rgba(0, 0, 0, var(--dim));
  }
</style>
