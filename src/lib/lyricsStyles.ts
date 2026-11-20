import type { LyricsFont, LyricsStyle } from './types';

export const LYRICS_STYLES: { id: LyricsStyle; name: string; hint: string }[] = [
  { id: 'classic', name: 'Classic', hint: 'Scrolling column next to the cover' },
  { id: 'spotlight', name: 'Spotlight', hint: 'One big line, neighbours above and below' },
  { id: 'poster', name: 'Poster', hint: 'Words stacked like a gig poster' },
  { id: 'bounce', name: 'Bounce', hint: 'Every letter hops as it is sung' },
  { id: 'terminal', name: 'Terminal', hint: 'Typed out line by line' },
  { id: 'subtitles', name: 'Subtitles', hint: 'Low on screen, made for video' },
];

export const LYRICS_FONTS: { id: LyricsFont; name: string }[] = [
  { id: 'sans', name: 'Sans' },
  { id: 'serif', name: 'Serif' },
  { id: 'mono', name: 'Mono' },
  { id: 'condensed', name: 'Condensed' },
  { id: 'rounded', name: 'Rounded' },
];

export const LYRICS_COLORS = ['#ffffff', '#ff4fa3', '#ffd23f', '#5cf2a6', '#4fc3ff', '#b28dff', '#ff7a45'];
