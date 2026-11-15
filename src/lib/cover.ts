function isYt3(url: string): boolean {
  return url.includes('yt3.ggpht.com') || url.includes('yt3.googleusercontent.com');
}

function isVideoId(id: string): boolean {
  return /^[\w-]{11}$/.test(id);
}

function coverUsable(url: string): boolean {
  return (
    url.startsWith('http') &&
    !url.includes('music.youtube.com/image/') &&
    !url.includes('/mixart')
  );
}

/** Public YouTube thumb - works in the webview without YTM cookies. */
export function ytThumb(id: string): string {
  return `https://i.ytimg.com/vi/${id}/hqdefault.jpg`;
}

/** Prefer API art; fall back to i.ytimg so the player bar is never blank. */
export function coverFor(t: { id: string; cover?: string | null } | null | undefined): string | null {
  if (!t?.id) return null;
  const c = t.cover;
  if (c && coverUsable(c)) return c;
  if (isVideoId(t.id)) return ytThumb(t.id);
  return c || null;
}

/** Rewrite YTM/Google CDN URLs to a requested pixel size (logical * DPR). */
export function sizedCover(url: string | null | undefined, w: number, h?: number): string {
  if (!url) return '';
  const width = Math.round(w);
  const height = Math.round(h ?? w);
  // Signed / special paths break if rewritten; mixart needs YTM cookies.
  if (
    url.includes('sqp=') ||
    url.includes('/pl_c/') ||
    url.includes('/s_p/') ||
    url.includes('/mixart') ||
    url.includes('music.youtube.com/image/')
  ) {
    return url;
  }
  // Channel avatars on yt3 only accept =sN-c-k-…; playlist/album art on
  // yt3 uses =wN-hN-l90-rj (avatar suffix 404s those).
  if (isYt3(url)) {
    const i = url.lastIndexOf('=');
    const base = i >= 0 ? url.slice(0, i) : url;
    if (url.includes('-c-k-') || /[=/]s\d+/.test(url)) {
      const s = Math.max(width, height);
      return `${base}=s${s}-c-k-c0x00ffffff-no-rj`;
    }
    return `${base}=w${width}-h${height}-l90-rj`;
  }
  if (url.includes('googleusercontent.com') || url.includes('ggpht.com')) {
    const i = url.lastIndexOf('=');
    if (i < 0) return url;
    const suffix = url.slice(i + 1);
    // Avatar-style size token - keep sN, don't force w/h.
    if (suffix.startsWith('s') && (suffix.includes('-c-k-') || suffix.includes('-c-c0x'))) {
      const s = Math.max(width, height);
      return `${url.slice(0, i)}=s${s}-c-k-c0x00ffffff-no-rj`;
    }
    // Cap DPR rewrites - huge w/h on small assets 404s for some album arts.
    const tw = Math.min(width, 544);
    const th = Math.min(height, 544);
    return `${url.slice(0, i)}=w${tw}-h${th}-l90-rj`;
  }
  if (url.includes('i.ytimg.com')) {
    // maxresdefault 404s for many videos - prefer hq which almost always exists.
    if (width >= 480) return swapYt(url, 'hqdefault');
    if (width >= 320) return swapYt(url, 'mqdefault');
  }
  return url;
}

function swapYt(url: string, name: string): string {
  return url.replace(
    /\/(default|mqdefault|hqdefault|sddefault|maxresdefault|hq720)\.jpg/,
    `/${name}.jpg`,
  );
}

/** srcset for a displayed CSS size in px - 1x, 2x, 3x. */
export function srcsetFor(url: string | null | undefined, cssPx: number, square = true): string {
  if (!url) return '';
  const h = square ? cssPx : Math.round(cssPx * 0.4);
  return [1, 2, 3]
    .map((d) => `${sizedCover(url, cssPx * d, square ? cssPx * d : h * d)} ${d}x`)
    .join(', ');
}

export function bannerSrcset(url: string | null | undefined, cssW: number, cssH: number): string {
  if (!url) return '';
  return [1, 2, 3]
    .map((d) => `${sizedCover(url, cssW * d, cssH * d)} ${d}x`)
    .join(', ');
}

export function itemToTrack(it: {
  id: string;
  title: string;
  artists?: string[];
  artist_ids?: string[];
  album?: string | null;
  album_id?: string | null;
  duration?: number | null;
  cover?: string | null;
  is_video?: boolean;
  kind?: string;
}): import('./types').Track {
  return {
    id: it.id,
    title: it.title,
    artists: it.artists ?? [],
    artist_ids: it.artist_ids ?? [],
    album: it.album ?? null,
    album_id: it.album_id ?? null,
    duration: it.duration ?? null,
    cover: it.cover ?? null,
    is_video: !!(it.is_video || it.kind === 'video'),
  };
}
