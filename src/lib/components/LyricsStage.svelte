<script lang="ts">
  import type { LyricLine, LyricsFont, LyricsStyle } from '../types';
  import { fitText, inkOn, posterWords } from '../fitText';

  type Style = Exclude<LyricsStyle, 'classic'>;

  let {
    lines,
    active,
    gap,
    gapP,
    progress,
    style,
    font = 'sans',
    upper = false,
    align = 'auto',
    color = null,
    scale = 1,
    karaoke = true,
    glow = true,
    title = '',
    artist = '',
    onseek,
  }: {
    lines: LyricLine[];
    /** Line being sung (-1: none yet). */
    active: number;
    /** Line a long instrumental gap is waiting for (-1: no gap). */
    gap: number;
    /** 0..1 through the current gap. */
    gapP: number;
    /** 0..1 through the active line. */
    progress: number;
    style: Style;
    font?: LyricsFont;
    upper?: boolean;
    align?: 'auto' | 'left' | 'center';
    /** Fixed highlight colour; null = the cover's accent. */
    color?: string | null;
    scale?: number;
    karaoke?: boolean;
    glow?: boolean;
    title?: string;
    artist?: string;
    onseek?: (ms: number) => void;
  } = $props();

  const has = (i: number) => i >= 0 && i < lines.length && !!lines[i].text.trim();

  /** Line in the big slot (-1 during a gap / before the first line). */
  const cur = $derived(active >= 0 && gap < 0 ? active : -1);
  /** Position everything else is relative to. */
  const anchor = $derived(cur >= 0 ? cur : gap >= 0 ? gap : active);
  const curText = $derived(cur >= 0 ? lines[cur]?.text.trim() ?? '' : '');

  const prevIdx = $derived.by(() => {
    const from = cur >= 0 ? cur - 1 : gap >= 0 ? gap - 1 : active;
    for (let i = from; i >= 0; i--) if (has(i)) return i;
    return -1;
  });
  const nextIdx = $derived.by(() => {
    const from = cur >= 0 ? cur + 1 : gap >= 0 ? gap : active + 1;
    for (let i = Math.max(0, from); i < lines.length; i++) if (has(i)) return i;
    return -1;
  });
  /** Terminal history: the lines already sung, oldest first. */
  const history = $derived.by(() => {
    const out: number[] = [];
    const from = cur >= 0 ? cur - 1 : gap >= 0 ? gap - 1 : active;
    for (let i = from; i >= 0 && out.length < 10; i--) if (has(i)) out.unshift(i);
    return out;
  });

  const words = $derived.by(() => {
    let off = 0;
    return curText
      .split(/\s+/)
      .filter(Boolean)
      .map((text) => {
        const chars = Array.from(text);
        const w = { text, chars, off, len: chars.length };
        off += chars.length + 1;
        return w;
      });
  });
  const nChars = $derived(words.length ? words[words.length - 1].off + words[words.length - 1].len : 1);
  /** Letters sung so far. */
  const head = $derived(karaoke ? progress * nChars : nChars + 1);
  const pwords = $derived(style === 'poster' ? posterWords(curText) : []);

  const wp = (off: number, len: number) => Math.min(1, Math.max(0, (head - off) / len));
  /** Fisheye: letters near the middle of the line are bigger. */
  const bulge = (k: number) => 0.72 + 0.58 * Math.sin(Math.PI * Math.min(1, Math.max(0, (k + 0.5) / nChars)));
  /** Neighbour lines for the fisheye drum, nearest first. */
  const around = (dir: -1 | 1) => {
    const out: number[] = [];
    let i = (cur >= 0 ? cur : gap >= 0 ? (dir < 0 ? gap : gap - 1) : active) + dir;
    for (; i >= 0 && i < lines.length && out.length < 2; i += dir) if (has(i)) out.push(i);
    return out;
  };
  const above = $derived(around(-1).reverse());
  const below = $derived(around(1));

  /** Little jump of the letter being sung right now (bounce). */
  const hop = (k: number) => (karaoke ? Math.max(0, 1 - Math.abs(head - k - 0.5) / 1.6) : 0);

  const slotKey = $derived(cur >= 0 ? `l${cur}` : gap >= 0 ? `g${gap}` : 'idle');
  const alignEff = $derived(
    style === 'poster' || style === 'subtitles'
      ? 'center'
      : align === 'auto'
        ? style === 'terminal' ? 'left' : 'center'
        : align,
  );
  const deps = $derived([curText, font, upper, scale, style]);

  const typed = $derived(
    cur < 0 ? '' : karaoke ? Array.from(curText).slice(0, Math.ceil(head)).join('') : curText,
  );

  function seek(i: number) {
    const ms = lines[i]?.start_ms;
    if (ms != null) onseek?.(ms);
  }
</script>

{#snippet sung()}
  {#each words as w, i (i)}
    {@const p = wp(w.off, w.len)}
    <span class="lw" class:on={p > 0} class:done={p >= 1} data-t={w.text} style:--wp="{(p * 100).toFixed(1)}%">{w.text}</span>{' '}
  {/each}
{/snippet}

{#snippet dots()}
  <span class="lx-dots" style:--gp={gap >= 0 ? gapP : 0} aria-hidden="true"><i></i><i></i><i></i></span>
{/snippet}

{#snippet bigSlot()}
  {#if cur >= 0 && curText}{@render sung()}{:else if cur >= 0}<span class="lx-note" aria-hidden="true">♪</span>{:else}{@render dots()}{/if}
{/snippet}

<div
  class="lx lx-{style}"
  class:upper
  class:glow
  class:left={alignEff === 'left'}
  class:custom={!!color}
  data-font={style === 'terminal' ? 'mono' : font}
  style:--lx-scale={scale}
  style:--lx-hl={color ?? 'var(--accent)'}
  style:--lx-ink={color ? inkOn(color) : 'var(--accent-fg, #111)'}
  role="region"
  aria-label="Lyrics"
>
  {#if style === 'spotlight'}
    <div class="sp-row sp-prev">
      {#if prevIdx >= 0}
        {#key prevIdx}
          <button type="button" class="sp-line lx-fade" onclick={() => seek(prevIdx)}>{lines[prevIdx].text}</button>
        {/key}
      {/if}
    </div>
    <div class="sp-box">
      {#key slotKey}
        <div class="sp-cur lx-in" use:fitText={{ max: (w, h) => Math.min(h * 0.46, w * 0.1, 132) * scale, min: 18, deps }}>
          {@render bigSlot()}
        </div>
      {/key}
    </div>
    <div class="sp-row sp-next">
      {#if nextIdx >= 0}
        {#key nextIdx}
          <button type="button" class="sp-line lx-fade" onclick={() => seek(nextIdx)}>{lines[nextIdx].text}</button>
        {/key}
      {/if}
    </div>
  {:else if style === 'fisheye'}
    <div class="fe-side fe-top">
      {#each above as i, k (i)}
        <button type="button" class="fe-line" style:--d={above.length - k} onclick={() => seek(i)}>{lines[i].text}</button>
      {/each}
    </div>
    <div class="fe-box">
      {#key slotKey}
        <div class="fe lx-in" use:fitText={{ max: (w, h) => Math.min(h * 0.32, w * 0.075, 104) * scale, min: 14, deps }}>
          {#if cur >= 0 && curText}
            {#each words as w, wi (wi)}
              <span class="fw">{#each w.chars as ch, ci (ci)}<span
                  class="fc"
                  class:on={head > w.off + ci}
                  style:--b={bulge(w.off + ci).toFixed(3)}
                >{ch}</span>{/each}</span>{' '}
            {/each}
          {:else}
            {@render bigSlot()}
          {/if}
        </div>
      {/key}
    </div>
    <div class="fe-side fe-bot">
      {#each below as i, k (i)}
        <button type="button" class="fe-line" style:--d={k + 1} onclick={() => seek(i)}>{lines[i].text}</button>
      {/each}
    </div>
  {:else if style === 'poster'}
    <div class="po-box">
      {#key slotKey}
        <div class="po lx-in" use:fitText={{ max: (w, h) => Math.min(h * 0.34, w * 0.15, 150) * scale, min: 16, deps }}>
          {#if cur >= 0 && curText}
            {#each pwords as w, i (i)}
              <span
                class="pw"
                class:hero={w.hero}
                class:alt={!w.hero && i % 3 === 1}
                class:on={wp(w.off, w.len) > 0}
                style:--w={w.w}
              >{w.text}</span>
            {/each}
          {:else}
            {@render bigSlot()}
          {/if}
        </div>
      {/key}
    </div>
  {:else if style === 'bounce'}
    <div class="bo-box">
      {#key slotKey}
        <div class="bo lx-in" use:fitText={{ max: (w, h) => Math.min(h * 0.4, w * 0.11, 140) * scale, min: 18, deps }}>
          {#if cur >= 0 && curText}
            {#each words as w, wi (wi)}
              <span class="bw">{#each w.chars as ch, ci (ci)}<span
                  class="bc"
                  class:on={head > w.off + ci}
                  style:--hop={hop(w.off + ci).toFixed(3)}
                  style:--k={w.off + ci}
                >{ch}</span>{/each}</span>{' '}
            {/each}
          {:else}
            {@render bigSlot()}
          {/if}
        </div>
      {/key}
    </div>
    <div class="bo-next">
      {#if nextIdx >= 0}
        {#key nextIdx}<button type="button" class="sp-line lx-fade" onclick={() => seek(nextIdx)}>{lines[nextIdx].text}</button>{/key}
      {/if}
    </div>
  {:else if style === 'terminal'}
    <div class="tm">
      <div class="tm-bar" aria-hidden="true"><i></i><i></i><i></i><span>{title}{artist ? ` — ${artist}` : ''}</span></div>
      <div class="tm-body">
        {#each history as i (i)}
          <button type="button" class="tl past" onclick={() => seek(i)}><span class="pr">›</span>{lines[i].text}</button>
        {/each}
        <div class="tl cur">
          <span class="pr">›</span>{#if cur >= 0}<span class="typed">{typed}</span>{:else if gap >= 0}<span class="cm">{'·'.repeat(1 + Math.floor(gapP * 3))}</span>{/if}<span class="caret" aria-hidden="true"></span>
        </div>
      </div>
    </div>
  {:else if style === 'subtitles'}
    <div class="sb-box">
      {#key slotKey}
        <div class="sb lx-in" use:fitText={{ max: (w, h) => Math.min(h * 0.5, w * 0.06, 72) * scale, min: 16, deps }}>
          {@render bigSlot()}
        </div>
      {/key}
    </div>
    <div class="sb-next">
      {#if nextIdx >= 0}
        {#key nextIdx}<span class="lx-fade">{lines[nextIdx].text}</span>{/key}
      {/if}
    </div>
  {/if}
</div>

<style>
  .lx {
    --lx-on: #fff;
    --lx-off: rgba(255, 255, 255, 0.3);
    --lx-glow: color-mix(in srgb, var(--lx-hl) 60%, transparent);
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    min-width: 0;
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    color: var(--lx-on);
    text-align: center;
    font-family: Inter, ui-sans-serif, system-ui, -apple-system, 'Segoe UI', sans-serif;
    user-select: none;
  }
  .lx.custom { --lx-on: var(--lx-hl); }
  .lx.left { text-align: left; }
  .lx.upper { text-transform: uppercase; }
  .lx[data-font='serif'] { font-family: 'Iowan Old Style', 'Palatino Linotype', Palatino, 'Book Antiqua', Georgia, 'Times New Roman', serif; }
  .lx[data-font='mono'] { font-family: ui-monospace, 'Cascadia Code', 'JetBrains Mono', 'SF Mono', Menlo, Consolas, 'DejaVu Sans Mono', monospace; }
  .lx[data-font='condensed'] { font-family: Bahnschrift, 'Arial Narrow', 'Roboto Condensed', 'Helvetica Neue', 'DejaVu Sans Condensed', sans-serif; font-stretch: 75%; }
  .lx[data-font='rounded'] { font-family: ui-rounded, 'SF Pro Rounded', Nunito, 'Varela Round', 'Arial Rounded MT Bold', ui-sans-serif, system-ui, sans-serif; }

  button { font: inherit; color: inherit; background: none; border: 0; padding: 0; text-align: inherit; text-transform: inherit; cursor: pointer; }

  /* Words of the big line fill left to right as they are sung. The filled
     copy sits on top and is clipped, so glow and shadows stay intact. */
  .lw {
    position: relative;
    display: inline-block;
    white-space: nowrap;
    color: var(--lx-off);
  }
  .lw::after {
    content: attr(data-t);
    position: absolute;
    left: 0;
    top: 0;
    color: var(--lx-on);
    clip-path: inset(-0.4em calc(100% - var(--wp)) -0.4em -0.4em);
    pointer-events: none;
  }
  .lw.done::after { clip-path: none; }
  .glow .lw.on::after { text-shadow: 0 0 0.32em var(--lx-glow); }
  :global(.fit-break) .lw,
  :global(.fit-break) .bw { white-space: normal; overflow-wrap: anywhere; }

  .lx-note { color: var(--lx-off); }
  .lx-dots { display: inline-flex; gap: calc(12px * var(--lx-scale)); vertical-align: middle; padding: 0.2em 0; }
  .lx-dots i {
    width: calc(14px * var(--lx-scale));
    height: calc(14px * var(--lx-scale));
    border-radius: 50%;
    background: var(--lx-on);
    opacity: 0.22;
    transition: opacity 0.3s, transform 0.3s;
  }
  .lx-dots i:nth-child(1) { opacity: calc(0.22 + 0.78 * clamp(0, var(--gp) * 3, 1)); }
  .lx-dots i:nth-child(2) { opacity: calc(0.22 + 0.78 * clamp(0, var(--gp) * 3 - 1, 1)); }
  .lx-dots i:nth-child(3) { opacity: calc(0.22 + 0.78 * clamp(0, var(--gp) * 3 - 2, 1)); }

  /* Entrance of a new line. `translate` (not `transform`) so it never
     fights the per-letter transforms below. */
  .lx-in { animation: lx-in 0.5s cubic-bezier(0.22, 1, 0.36, 1) backwards; }
  .lx-fade { animation: lx-fade 0.45s ease backwards; }
  @keyframes lx-in {
    from { opacity: 0; translate: 0 0.3em; }
  }
  @keyframes lx-fade {
    from { opacity: 0; }
  }

  /* ---------- spotlight: one big line, its neighbours whispering ---------- */
  .sp-row {
    flex: none;
    height: calc(clamp(16px, 1.9vw, 26px) * var(--lx-scale) * 1.5);
    font-size: calc(clamp(16px, 1.9vw, 26px) * var(--lx-scale));
    line-height: 1.5;
    font-weight: 600;
    overflow: hidden;
  }
  .sp-line {
    display: block;
    width: 100%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: rgba(255, 255, 255, 0.38);
    transition: color 0.2s;
  }
  .sp-line:hover { color: rgba(255, 255, 255, 0.7); }
  .sp-box {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 0.4em 0;
  }
  .sp-cur {
    flex: none;
    width: 100%;
    font-weight: 750;
    line-height: 1.12;
    letter-spacing: -0.02em;
  }
  .lx-spotlight .sp-prev { margin-bottom: 4px; }

  /* ---------- fisheye: the line bulges, neighbours curve away ---------- */
  .fe-side {
    flex: none;
    display: flex;
    flex-direction: column;
    perspective: 520px;
    font-size: calc(clamp(15px, 1.8vw, 26px) * var(--lx-scale));
    line-height: 1.45;
    font-weight: 700;
  }
  .fe-line {
    display: block;
    width: 100%;
    height: 1.45em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: rgba(255, 255, 255, calc(0.5 - var(--d) * 0.15));
    /* Tilted away like the edge of a lens; the edge nearest the big line
       stays put, so the tilt can only shrink the line inside its own row. */
    transform: rotateX(calc(var(--d) * 26deg)) scale(calc(1 - var(--d) * 0.1));
    transition: color 0.2s;
  }
  .fe-top .fe-line { transform-origin: 50% 100%; }
  .fe-bot .fe-line { transform-origin: 50% 0%; transform: rotateX(calc(var(--d) * -26deg)) scale(calc(1 - var(--d) * 0.1)); }
  .fe-line:hover { color: rgba(255, 255, 255, 0.75); }
  .fe-box {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 0.3em 0;
  }
  .fe {
    flex: none;
    width: 100%;
    font-weight: 800;
    line-height: 1.12;
    letter-spacing: -0.01em;
  }
  .fw { white-space: nowrap; }
  /* Real font sizes (not transforms): the bulge takes up layout space, so
     bigger letters push their neighbours instead of covering them. */
  .fc {
    display: inline-block;
    font-size: calc(var(--b) * 1em);
    color: var(--lx-off);
    transition: color 0.18s ease;
  }
  .fc.on { color: var(--lx-on); }
  .glow .fc.on { text-shadow: 0 0 0.3em var(--lx-glow); }

  /* ---------- poster: words stacked like a gig poster ---------- */
  .po-box,
  .bo-box,
  .sb-box {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
  }
  .po {
    flex: none;
    width: 100%;
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    align-items: baseline;
    column-gap: 0.24em;
    row-gap: 0.1em;
    padding: 0.1em 0;
    line-height: 0.98;
    font-weight: 850;
    letter-spacing: -0.03em;
  }
  .pw {
    font-size: calc(var(--w) * 1em);
    white-space: nowrap;
    color: #fff;
    opacity: 0.13;
    transform: translateY(0.14em) scale(0.92);
    transition:
      opacity 0.18s ease,
      transform 0.42s cubic-bezier(0.34, 1.56, 0.64, 1);
  }
  .pw.on { opacity: 1; transform: none; }
  .pw.alt { color: var(--lx-hl); }
  .pw.hero {
    background: var(--lx-hl);
    color: var(--lx-ink);
    padding: 0.02em 0.14em 0.08em;
    border-radius: 0.1em;
  }
  .pw.hero.on { transform: rotate(-1.5deg); }
  .glow .pw.on:not(.hero) { text-shadow: 0 0 0.28em var(--lx-glow); }
  .glow .pw.hero.on { box-shadow: 0 0 0.5em var(--lx-glow); }

  /* ---------- bounce: each letter hops as it is sung ---------- */
  .bo {
    flex: none;
    width: 100%;
    padding-top: 0.2em;
    font-weight: 800;
    /* Room between rows so a hopping letter never touches the row above. */
    line-height: 1.42;
    letter-spacing: -0.01em;
  }
  .bw { white-space: nowrap; }
  .bc {
    display: inline-block;
    color: var(--lx-off);
    transform: translateY(calc(var(--hop) * -0.16em)) scale(calc(1 + var(--hop) * 0.06));
    transition: color 0.18s ease;
    animation: lx-letter 0.5s cubic-bezier(0.22, 1, 0.36, 1) backwards;
    animation-delay: calc(min(var(--k), 40) * 12ms);
  }
  .bc.on { color: var(--lx-on); }
  .lx:not(.custom) .bc.on { color: color-mix(in srgb, var(--lx-hl) 35%, #fff); }
  .glow .bc.on { text-shadow: 0 0 0.3em var(--lx-glow); }
  @keyframes lx-letter {
    from { opacity: 0; translate: 0 0.5em; }
  }
  .bo-next {
    flex: none;
    margin-top: 10px;
    height: calc(clamp(16px, 1.8vw, 24px) * var(--lx-scale) * 1.5);
    font-size: calc(clamp(16px, 1.8vw, 24px) * var(--lx-scale));
    line-height: 1.5;
    font-weight: 600;
    overflow: hidden;
  }

  /* ---------- terminal: typed out, line by line ---------- */
  .lx-terminal { justify-content: flex-end; }
  .tm {
    /* Grows upwards with the history, up to the whole stage. */
    flex: 0 1 auto;
    min-height: min(100%, 52%);
    max-height: 100%;
    width: min(100%, 1100px);
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    border-radius: 16px;
    overflow: hidden;
    background: rgba(10, 10, 12, 0.62);
    border: 1px solid rgba(255, 255, 255, 0.08);
    box-shadow: 0 24px 64px -20px rgba(0, 0, 0, 0.6);
    font-size: calc(clamp(20px, 2.7vw, 46px) * var(--lx-scale));
    text-align: left;
  }
  .tm-bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 38px;
    padding: 0 14px;
    font-size: 12px;
    line-height: 1;
    color: rgba(255, 255, 255, 0.45);
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    text-transform: none;
  }
  .tm-bar i { width: 11px; height: 11px; border-radius: 50%; background: rgba(255, 255, 255, 0.16); flex: none; }
  .tm-bar span { margin-left: 8px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tm-body {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    gap: 0.2em;
    padding: 0.6em 0.9em 0.8em;
    overflow: hidden;
    line-height: 1.35;
    -webkit-mask-image: linear-gradient(to bottom, transparent 0%, #000 30%);
    mask-image: linear-gradient(to bottom, transparent 0%, #000 30%);
  }
  .tl {
    flex: none;
    display: block;
    width: 100%;
    overflow-wrap: anywhere;
    text-align: left;
  }
  .tl.past { color: rgba(255, 255, 255, 0.38); font-size: 0.66em; }
  .tl.past:hover { color: rgba(255, 255, 255, 0.7); }
  .pr { color: var(--lx-hl); margin-right: 0.6em; font-weight: 700; }
  .tl.cur { color: var(--lx-on); font-weight: 600; }
  .glow .tl.cur .typed { text-shadow: 0 0 0.35em var(--lx-glow); }
  .cm { color: rgba(255, 255, 255, 0.4); letter-spacing: 0.2em; }
  .caret {
    display: inline-block;
    width: 0.55em;
    height: 1.05em;
    margin-left: 0.08em;
    vertical-align: -0.16em;
    background: var(--lx-hl);
    animation: lx-blink 1s steps(1) infinite;
  }
  @keyframes lx-blink {
    50% { opacity: 0; }
  }

  /* ---------- subtitles: low on the screen, made for a video behind ---------- */
  .lx-subtitles { justify-content: flex-end; }
  .sb-box {
    flex: 0 1 38%;
    justify-content: flex-end;
    width: min(100%, 1200px);
    margin: 0 auto;
  }
  .sb {
    flex: none;
    width: 100%;
    font-weight: 700;
    line-height: 1.2;
    filter: drop-shadow(0 2px 6px rgba(0, 0, 0, 0.85)) drop-shadow(0 0 1px rgba(0, 0, 0, 0.9));
  }
  .lx-subtitles .lw { color: rgba(255, 255, 255, 0.55); }
  .sb-next {
    flex: none;
    height: calc(clamp(15px, 1.6vw, 22px) * var(--lx-scale) * 1.6);
    margin-top: 6px;
    font-size: calc(clamp(15px, 1.6vw, 22px) * var(--lx-scale));
    line-height: 1.6;
    font-weight: 600;
    color: rgba(255, 255, 255, 0.5);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    filter: drop-shadow(0 1px 4px rgba(0, 0, 0, 0.8));
  }

  @media (prefers-reduced-motion: reduce) {
    .lx-in, .lx-fade, .bc { animation: none; }
    .bc { transform: none; }
    .caret { animation: none; }
  }
</style>
