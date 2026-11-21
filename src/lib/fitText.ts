/**
 * Svelte action: give `node` the largest font size at which it fits inside
 * its parent's content box (no horizontal overflow, no taller than the box).
 *
 * Words never get split unless even the minimum size overflows; then the
 * node gets the `fit-break` class so CSS can let long words wrap.
 * Layout-only measurement: transforms / animations on the text don't matter.
 */
export interface FitOpts {
  /** Largest size in px for a box of this width / height. */
  max: (w: number, h: number) => number;
  /** Smallest size in px (default 14). */
  min?: number;
  /** Anything that changes the text's layout (text, font, case, scale...). */
  deps?: unknown;
  /** Runs before measuring (e.g. to drop transforms that would skew it). */
  before?: () => void;
  /** Runs after every fit, with the final size applied. */
  after?: () => void;
}

export function fitText(node: HTMLElement, opts: FitOpts) {
  let o = opts;
  let raf = 0;
  const box = node.parentElement;

  function run() {
    raf = 0;
    o.before?.();
    fit();
    o.after?.();
  }

  function fit() {
    if (!box) return;
    const cs = getComputedStyle(box);
    const aw = box.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
    const ah = box.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom);
    if (aw < 20 || ah < 10) return;
    const min = o.min ?? 14;
    let lo = min;
    let hi = Math.max(min, o.max(aw, ah));
    node.classList.remove('fit-break');
    const fits = (px: number) => {
      node.style.fontSize = `${px}px`;
      return node.scrollWidth <= node.clientWidth + 1 && node.offsetHeight <= ah + 1;
    };
    if (fits(hi)) return;
    if (!fits(lo)) {
      node.classList.add('fit-break');
      // Even broken up it must not be taller than the box.
      while (lo > 8 && !fits(lo)) lo -= 2;
      return;
    }
    for (let i = 0; i < 9 && hi - lo > 0.5; i++) {
      const mid = (lo + hi) / 2;
      if (fits(mid)) lo = mid;
      else hi = mid;
    }
    node.style.fontSize = `${Math.floor(lo)}px`;
  }

  const schedule = () => {
    if (!raf) raf = requestAnimationFrame(run);
  };
  const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(schedule) : null;
  if (box) ro?.observe(box);
  // Web fonts can arrive after the first measurement.
  document.fonts?.ready.then(schedule).catch(() => {});
  schedule();

  return {
    update(n: FitOpts) {
      o = n;
      schedule();
    },
    destroy() {
      ro?.disconnect();
      if (raf) cancelAnimationFrame(raf);
    },
  };
}

/** Words that carry little weight in a lyric line (rendered smaller on the poster). */
const SMALL = new Set(
  (
    'a an the and or but so of to in on at by for from with as is am are was were be been ' +
    'i im i\'m me my you your u ya we us our he him his she her it its they them their ' +
    'this that these those just do dont don\'t can cant can\'t will wont won\'t oh ooh ah yeah ' +
    'si e și sa să la de pe cu in în nu mi ma mă te tu eu el ea ce un o'
  ).split(' '),
);

export interface PosterWord {
  text: string;
  /** Size relative to the line's base size. */
  w: number;
  hero: boolean;
  /** First letter index in the line (for karaoke timing). */
  off: number;
  len: number;
}

/** Split a line into words with poster sizes; one "hero" word gets a box. */
export function posterWords(text: string): PosterWord[] {
  let off = 0;
  const words = text
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .map((t) => {
      const len = Array.from(t).length;
      const o = off;
      off += len + 1;
      const key = t.toLowerCase().replace(/[^\p{L}']/gu, '');
      const small = SMALL.has(key);
      const w = small ? 0.62 : len <= 3 ? 1.1 : len <= 7 ? 1 : 0.86;
      return { text: t, w, hero: false, off: o, len, small };
    });
  // Hero: the longest word that is not a filler word (the last one on ties).
  let best = -1;
  for (let i = 0; i < words.length; i++) {
    if (words[i].small) continue;
    if (best < 0 || words[i].len >= words[best].len) best = i;
  }
  if (best >= 0 && words.length > 1) {
    words[best].hero = true;
    words[best].w = Math.max(words[best].w, 1.18);
  }
  return words.map(({ small: _s, ...w }) => w);
}

/** Readable text colour on top of `hex`. */
export function inkOn(hex: string): string {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return '#111';
  const n = parseInt(m[1], 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  const lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  return lum > 0.55 ? '#111' : '#fff';
}
