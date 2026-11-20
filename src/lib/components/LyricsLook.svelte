<script lang="ts">
  import { t as tr } from '../i18n.svelte';
  import Hi from './Hi.svelte';
  import Toggle from './Toggle.svelte';
  import Slider from './Slider.svelte';
  import { SubtitleIcon, Cancel01Icon } from '../icons';
  import type { Settings } from '../types';
  import { LYRICS_COLORS, LYRICS_FONTS, LYRICS_STYLES } from '../lyricsStyles';

  let {
    settings,
    onsave,
    placement = 'up',
  }: {
    settings: Settings;
    onsave: () => void;
    placement?: 'up' | 'down';
  } = $props();

  let open = $state(false);
  let root = $state<HTMLElement | null>(null);
  let popEl = $state<HTMLElement | null>(null);
  let popPos = $state<{ top?: string; bottom?: string; right: string; maxH: string } | null>(null);

  const STYLES = LYRICS_STYLES;
  const FONTS = LYRICS_FONTS;
  const COLORS = LYRICS_COLORS;

  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }

  function placePop() {
    if (!root) return;
    const r = (root.querySelector('button') ?? root).getBoundingClientRect();
    const right = `${Math.max(8, window.innerWidth - r.right)}px`;
    popPos = placement === 'up'
      ? { bottom: `${Math.max(8, window.innerHeight - r.top + 10)}px`, right, maxH: `${Math.max(200, r.top - 22)}px` }
      : { top: `${r.bottom + 8}px`, right, maxH: `${Math.max(200, window.innerHeight - r.bottom - 22)}px` };
  }

  $effect(() => {
    if (!open) return;
    let ready = false;
    const arm = window.setTimeout(() => { ready = true; }, 0);
    const onDoc = (e: PointerEvent) => {
      if (!ready) return;
      const t = e.target as Node;
      if (root?.contains(t) || popEl?.contains(t)) return;
      open = false;
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') open = false;
    };
    placePop();
    document.addEventListener('pointerdown', onDoc, true);
    document.addEventListener('keydown', onKey);
    window.addEventListener('resize', placePop);
    return () => {
      window.clearTimeout(arm);
      document.removeEventListener('pointerdown', onDoc, true);
      document.removeEventListener('keydown', onKey);
      window.removeEventListener('resize', placePop);
    };
  });

  const style = $derived(settings.lyrics_style ?? 'classic');
  const font = $derived(settings.lyrics_font ?? 'sans');
  const align = $derived(settings.lyrics_align ?? 'auto');

  function set<K extends keyof Settings>(k: K, v: Settings[K]) {
    settings[k] = v;
    onsave();
  }
</script>

<div class="ll" bind:this={root}>
  <button
    class="ibtn lg"
    type="button"
    class:on={open}
    title={tr('Lyrics look')}
    aria-label={tr('Lyrics look')}
    aria-expanded={open}
    aria-haspopup="dialog"
    onclick={(e) => { e.stopPropagation(); open = !open; }}
  >
    <Hi icon={SubtitleIcon} size={22} />
  </button>
  {#if open}
    <div
      class="ll-pop"
      role="dialog"
      aria-label={tr('Lyrics look')}
      use:portal
      bind:this={popEl}
      style:position="fixed"
      style:top={popPos?.top}
      style:bottom={popPos?.bottom}
      style:right={popPos?.right}
      style:max-height={popPos?.maxH}
    >
      <div class="ll-h">
        <div class="ll-title">{tr('Lyrics look')}</div>
        <button class="ll-x" type="button" aria-label={tr('Close')} onclick={() => (open = false)}>
          <Hi icon={Cancel01Icon} size={18} />
        </button>
      </div>

      <div class="ll-lab">{tr('Style')}</div>
      <div class="ll-styles" role="radiogroup" aria-label={tr('Style')}>
        {#each STYLES as s (s.id)}
          <button
            type="button"
            role="radio"
            aria-checked={style === s.id}
            class="ll-style st-{s.id}"
            class:on={style === s.id}
            title={tr(s.hint)}
            onclick={() => set('lyrics_style', s.id)}
          >
            <span class="ll-prev" aria-hidden="true">
              {#if s.id === 'classic'}<i></i><b>Aa</b><i></i>
              {:else if s.id === 'spotlight'}<i></i><b>Aa</b><i></i>
              {:else if s.id === 'fisheye'}<i class="t"></i><b><small>a</small>A<small>a</small></b><i class="b"></i>
              {:else if s.id === 'poster'}<b>Aa</b><em>BB</em>
              {:else if s.id === 'bounce'}<b>A</b><b class="up">a</b><b>a</b>
              {:else if s.id === 'terminal'}<b>›_</b>
              {:else}<b>Aa</b>{/if}
            </span>
            <span class="ll-name">{tr(s.name)}</span>
          </button>
        {/each}
      </div>

      <div class="ll-lab">{tr('Font')}</div>
      <div class="ll-chips">
        {#each FONTS as f (f.id)}
          <button
            type="button"
            class="ll-chip ff-{f.id}"
            class:on={font === f.id}
            disabled={style === 'terminal'}
            onclick={() => set('lyrics_font', f.id)}
          >{tr(f.name)}</button>
        {/each}
      </div>

      <div class="ll-lab">{tr('Alignment')}</div>
      <div class="ll-chips">
        {#each [['auto', 'Auto'], ['left', 'Left'], ['center', 'Center']] as [v, n] (v)}
          <button
            type="button"
            class="ll-chip"
            class:on={align === v}
            disabled={style === 'poster' || style === 'subtitles'}
            onclick={() => set('lyrics_align', v as 'auto' | 'left' | 'center')}
          >{tr(n)}</button>
        {/each}
      </div>

      <div class="ll-lab">{tr('Colour')}</div>
      <div class="ll-colors">
        <button
          type="button"
          class="ll-sw auto"
          class:on={!settings.lyrics_color}
          title={tr('From cover art')}
          aria-label={tr('From cover art')}
          onclick={() => set('lyrics_color', null)}
        ></button>
        {#each COLORS as c (c)}
          <button
            type="button"
            class="ll-sw"
            class:on={settings.lyrics_color === c}
            style:background={c}
            title={c}
            aria-label={c}
            onclick={() => set('lyrics_color', c)}
          ></button>
        {/each}
        <label
          class="ll-sw custom"
          class:on={!!settings.lyrics_color && !COLORS.includes(settings.lyrics_color)}
          title={tr('Custom colour')}
          style:background={settings.lyrics_color && !COLORS.includes(settings.lyrics_color) ? settings.lyrics_color : null}
        >
          <input
            type="color"
            value={settings.lyrics_color ?? '#ff4fa3'}
            onchange={(e) => set('lyrics_color', (e.currentTarget as HTMLInputElement).value)}
          />
        </label>
      </div>

      <div class="ll-row">
        <span>{tr('Uppercase')}</span>
        <Toggle checked={!!settings.lyrics_upper} label={tr('Uppercase')} onchange={(v) => set('lyrics_upper', v)} />
      </div>
      <div class="ll-row">
        <span>{tr('Text size')} <em>{Math.round(settings.lyrics_size)}%</em></span>
        <div class="ll-sl">
          <Slider
            value={settings.lyrics_size - 70}
            max={80}
            live={true}
            label={tr('Text size')}
            oninput={(v) => { settings.lyrics_size = Math.round(v) + 70; }}
            onchange={(v) => set('lyrics_size', Math.round(v) + 70)}
          />
        </div>
      </div>

      <div class="ll-sep"></div>
      <div class="ll-row">
        <span>{tr('Music video in the background')}
          <small>{tr('When the song has one. Downloads the video (no sound)')}</small>
        </span>
        <Toggle checked={!!settings.bg_video} label={tr('Music video in the background')} onchange={(v) => set('bg_video', v)} />
      </div>
      {#if settings.bg_video}
        <div class="ll-row">
          <span>{tr('Darken video')} <em>{Math.round(settings.bg_video_dim ?? 35)}%</em></span>
          <div class="ll-sl">
            <Slider
              value={settings.bg_video_dim ?? 35}
              max={80}
              live={true}
              label={tr('Darken video')}
              oninput={(v) => { settings.bg_video_dim = Math.round(v); }}
              onchange={(v) => set('bg_video_dim', Math.round(v))}
            />
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .ll { position: relative; flex: 0 0 auto; }
  .ll-pop {
    width: min(360px, calc(100vw - 24px));
    overflow-y: auto;
    padding: 14px;
    border-radius: 16px;
    border: 1px solid var(--line-2);
    background: color-mix(in srgb, var(--bg-elev) 96%, #0c0b0a 4%);
    box-shadow: var(--shadow);
    z-index: 10060;
    scrollbar-width: thin;
  }
  .ll-h { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; }
  .ll-title { font-size: 15px; font-weight: 650; letter-spacing: -0.01em; }
  .ll-x { width: 28px; height: 28px; border-radius: 8px; display: grid; place-items: center; color: var(--text-3); }
  .ll-x:hover { background: var(--bg-hover); color: var(--text); }
  .ll-lab {
    margin: 12px 0 6px;
    font-size: 11px;
    font-weight: 650;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-3);
  }
  .ll-styles { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 6px; }
  .ll-style {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
    padding: 6px;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: var(--bg);
    color: var(--text-2);
    min-width: 0;
  }
  .ll-style:hover { border-color: var(--line-2); color: var(--text); }
  .ll-style.on { border-color: rgba(var(--accent-rgb), 0.85); color: var(--text); box-shadow: 0 0 0 1px rgba(var(--accent-rgb), 0.5) inset; }
  .ll-prev {
    height: 46px;
    border-radius: 8px;
    background: linear-gradient(160deg, #2b2622, #151311);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    overflow: hidden;
    color: #fff;
  }
  .ll-prev b { font-weight: 800; font-size: 17px; line-height: 1; }
  .ll-prev i { display: block; width: 46%; height: 3px; border-radius: 2px; background: rgba(255, 255, 255, 0.25); }
  .st-classic .ll-prev { align-items: flex-start; padding-left: 10px; }
  .st-classic .ll-prev b { font-size: 13px; }
  .st-spotlight .ll-prev b { font-size: 20px; }
  .st-fisheye .ll-prev { perspective: 80px; }
  .st-fisheye .ll-prev b { font-size: 22px; }
  .st-fisheye .ll-prev small { font-size: 13px; }
  .st-fisheye .ll-prev .t { transform: rotateX(40deg); }
  .st-fisheye .ll-prev .b { transform: rotateX(-40deg); }
  .st-poster .ll-prev { flex-direction: row; gap: 4px; }
  .st-poster .ll-prev em { font-style: normal; font-weight: 900; font-size: 12px; padding: 1px 3px; border-radius: 3px; background: rgb(var(--accent-rgb)); color: var(--accent-fg, #111); transform: rotate(-4deg); }
  .st-bounce .ll-prev { flex-direction: row; gap: 1px; }
  .st-bounce .ll-prev .up { transform: translateY(-5px); color: rgb(var(--accent-rgb)); }
  .st-terminal .ll-prev { align-items: flex-start; padding-left: 8px; font-family: ui-monospace, Menlo, Consolas, monospace; }
  .st-terminal .ll-prev b { color: rgb(var(--accent-rgb)); font-size: 14px; }
  .st-subtitles .ll-prev { justify-content: flex-end; padding-bottom: 6px; }
  .st-subtitles .ll-prev b { font-size: 12px; }
  .ll-name { font-size: 12px; font-weight: 600; text-align: center; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .ll-chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .ll-chip {
    padding: 6px 10px;
    border-radius: 999px;
    border: 1px solid var(--line);
    font-size: 13px;
    color: var(--text-2);
  }
  .ll-chip:hover:not(:disabled) { color: var(--text); border-color: var(--line-2); }
  .ll-chip.on { color: var(--accent-fg, #111); background: rgb(var(--accent-rgb)); border-color: transparent; }
  .ll-chip:disabled { opacity: 0.4; cursor: default; }
  .ff-serif { font-family: 'Iowan Old Style', 'Palatino Linotype', Georgia, serif; }
  .ff-mono { font-family: ui-monospace, 'Cascadia Code', Menlo, Consolas, monospace; }
  .ff-condensed { font-family: Bahnschrift, 'Arial Narrow', 'Roboto Condensed', sans-serif; font-stretch: 75%; }
  .ff-rounded { font-family: ui-rounded, 'SF Pro Rounded', Nunito, 'Varela Round', sans-serif; }
  .ll-colors { display: flex; flex-wrap: wrap; gap: 8px; }
  .ll-sw {
    position: relative;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    border: 1px solid rgba(255, 255, 255, 0.18);
    cursor: pointer;
    overflow: hidden;
  }
  .ll-sw.on { box-shadow: 0 0 0 2px var(--bg-elev), 0 0 0 4px var(--text); }
  .ll-sw.auto { background: conic-gradient(from 200deg, rgb(var(--accent-rgb)), #fff, rgb(var(--accent-rgb))); }
  .ll-sw.custom { background: conic-gradient(red, yellow, lime, cyan, blue, magenta, red); }
  .ll-sw input { position: absolute; inset: 0; opacity: 0; cursor: pointer; }
  .ll-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-height: 42px;
    margin-top: 8px;
    font-size: 14px;
    font-weight: 550;
  }
  .ll-row > span { display: flex; flex-direction: column; min-width: 0; }
  .ll-row em { font-style: normal; color: var(--text-3); font-weight: 500; margin-left: 4px; }
  .ll-row small { font-size: 11px; line-height: 15px; color: var(--text-3); font-weight: 500; margin-top: 2px; }
  .ll-sl { width: 140px; flex: none; }
  .ll-sep { height: 1px; background: var(--line); margin: 12px 0 4px; }
</style>
