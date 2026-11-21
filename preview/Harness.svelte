<script lang="ts">
  import LyricsStage from '../src/lib/components/LyricsStage.svelte';
  import type { LyricLine, LyricsFont, LyricsStyle } from '../src/lib/types';

  const q = new URLSearchParams(location.search);
  const style = (q.get('style') ?? 'spotlight') as Exclude<LyricsStyle, 'classic'>;
  const font = (q.get('font') ?? 'sans') as LyricsFont;
  const upper = q.has('upper');
  const align = (q.get('align') ?? 'auto') as 'auto' | 'left' | 'center';
  const color = q.get('color') ? `#${q.get('color')}` : null;
  const scale = Number(q.get('scale') ?? '1');
  const idle = q.has('idle');
  const queue = q.has('queue');

  const texts = [
    'Hold me closer, the night is young',
    'I know you love me',
    'Even when the city lights go down and every single window burns away',
    'Supercalifragilisticexpialidocious',
    'Și dacă plouă, eu tot dansez în ploaie cu tine',
    '',
    'Oh',
    'We were young and we were wild, we never thought about tomorrow or the day after that',
  ];
  const lines: LyricLine[] = texts.map((text, i) => ({ text, start_ms: i * 4000 }));
  let active = $state(Number(q.get('line') ?? '1'));
  if (q.has('step')) setTimeout(() => (active += 1), 1000);
  const gap = Number(q.get('gap') ?? '-1');
  const progress = Number(q.get('p') ?? '0.6');
</script>

<div class="np" class:idle class:queue>
  <div class="bg"></div>
  {#if !idle}<div class="close" data-chrome>X</div>{/if}
  {#if queue}<div class="qcard" data-chrome></div>{/if}
  <div class="np-in">
    <div class="lyx-head" data-chrome>
      <div class="lyx-art"></div>
      <div class="lyx-meta"><b>Song title that is quite long indeed</b><span>Artist Name</span></div>
    </div>
    <LyricsStage
      {lines}
      {active}
      {gap}
      gapP={0.5}
      {progress}
      {style}
      {font}
      {upper}
      {align}
      {color}
      {scale}
      karaoke={true}
      glow={true}
      title="Song title"
      artist="Artist"
    />
  </div>
  {#if !idle}<div class="dock" data-chrome>dock</div>{/if}
</div>

<style>
  :global(:root) {
    --accent: #e0a45a;
    --accent-fg: #1a1208;
    --accent-rgb: 224, 164, 90;
    --ease: cubic-bezier(0.22, 1, 0.36, 1);
  }
  :global(html, body) { margin: 0; height: 100%; background: #111; color: #fff; overflow: hidden; }
  :global(*) { box-sizing: border-box; }
  .np { position: fixed; inset: 0; overflow: hidden; }
  .bg { position: absolute; inset: 0; background: radial-gradient(circle at 30% 30%, #6a2a5a, #1b1020 70%); }
  .np-in {
    position: relative;
    z-index: 1;
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: clamp(10px, 1.6vh, 18px);
    padding: calc(36px + 14px) clamp(24px, 5vw, 80px) calc(128px + 8px);
  }
  .queue .np-in { padding-right: min(340px, 36vw); }
  .idle .np-in { padding: 28px clamp(24px, 5vw, 80px) 32px; }
  .idle.queue .np-in { padding-right: min(340px, 36vw); }
  .lyx-head { flex: none; display: flex; align-items: center; gap: 12px; max-width: calc(100% - 72px); height: 52px; overflow: hidden; }
  .idle .lyx-head { height: 0; opacity: 0; }
  .lyx-art { width: 52px; height: 52px; border-radius: 10px; background: #a66; flex: none; }
  .lyx-meta { display: flex; flex-direction: column; min-width: 0; font-family: sans-serif; }
  .lyx-meta b, .lyx-meta span { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .close { position: absolute; top: 48px; right: 16px; width: 48px; height: 48px; z-index: 5; background: rgba(255, 0, 0, 0.25); }
  .qcard { position: absolute; top: 56px; right: 20px; bottom: 144px; width: min(300px, 32vw); z-index: 4; background: rgba(0, 120, 255, 0.25); }
  .idle .qcard { bottom: 24px; }
  .dock { position: absolute; left: 0; right: 0; bottom: 0; height: 128px; z-index: 6; background: rgba(0, 255, 0, 0.12); }
</style>
