export function fmtTime(s: number | null | undefined): string {
  if (s == null || !isFinite(s) || s < 0) return '0:00';
  const sec = Math.floor(s);
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const r = sec % 60;
  const mm = h > 0 ? String(m).padStart(2, '0') : String(m);
  const ss = String(r).padStart(2, '0');
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

export function fmtCount(n: number | null | undefined): string {
  if (n == null) return '';
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1).replace(/\.0$/, '')}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, '')}K`;
  return String(n);
}

export function artistsLine(a: string[] | undefined): string {
  return (a ?? []).filter(Boolean).join(', ');
}

export function shareUrl(opts: { video?: string; album?: string; playlist?: string; artist?: string }): string {
  if (opts.video) return `https://music.youtube.com/watch?v=${opts.video}`;
  if (opts.album) return `https://music.youtube.com/browse/${opts.album}`;
  if (opts.playlist) return `https://music.youtube.com/playlist?list=${opts.playlist}`;
  if (opts.artist) return `https://music.youtube.com/channel/${opts.artist}`;
  return 'https://music.youtube.com/';
}
