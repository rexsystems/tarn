function isYt3(url: string): boolean {
  return url.includes('yt3.ggpht.com') || url.includes('yt3.googleusercontent.com');
}

/** Rewrite YTM/Google CDN URLs to a requested pixel size (logical * DPR). */
export function sizedCover(url: string | null | undefined, w: number, h?: number): string {
  if (!url) return '';
  const width = Math.round(w);
  const height = Math.round(h ?? w);
  // Channel avatars on yt3 only accept =sN-c-k-…; the album-art
  // =wN-hN-l90-rj suffix 404s and shows as a broken image.
  if (isYt3(url)) {
    const i = url.lastIndexOf('=');
    const base = i >= 0 ? url.slice(0, i) : url;
    const s = Math.max(width, height);
    return `${base}=s${s}-c-k-c0x00ffffff-no-rj`;
  }
  if (url.includes('googleusercontent.com') || url.includes('ggpht.com')) {
    const i = url.lastIndexOf('=');
    if (i >= 0) return `${url.slice(0, i)}=w${width}-h${height}-l90-rj`;
  }
  if (url.includes('i.ytimg.com')) {
    if (width >= 1280) return swapYt(url, 'maxresdefault');
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

/** srcset for a displayed CSS size in px — 1x, 2x, 3x. */
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
