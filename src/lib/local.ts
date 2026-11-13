import type { Track } from './types';

const LIKES_KEY = 'tarn.likes.v1';
const RECENTS_KEY = 'tarn.recents.v1';
const MAX_LIKES = 500;
const MAX_RECENTS = 80;

function read<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return fallback;
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

function write(key: string, v: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(v));
  } catch {
    /* quota */
  }
}

export function localLikes(): Track[] {
  const v = read<Track[]>(LIKES_KEY, []);
  return Array.isArray(v) ? v.filter((t) => t && t.id) : [];
}

export function isLocalLiked(id: string): boolean {
  return !!id && localLikes().some((t) => t.id === id);
}

export function setLocalLiked(track: Track, on: boolean): Track[] {
  let list = localLikes().filter((t) => t.id !== track.id);
  if (on) list = [track, ...list].slice(0, MAX_LIKES);
  write(LIKES_KEY, list);
  return list;
}

export function localRecents(): Track[] {
  const v = read<Track[]>(RECENTS_KEY, []);
  return Array.isArray(v) ? v.filter((t) => t && t.id) : [];
}

export function recordRecent(track: Track): Track[] {
  if (!track?.id) return localRecents();
  const list = [track, ...localRecents().filter((t) => t.id !== track.id)].slice(0, MAX_RECENTS);
  write(RECENTS_KEY, list);
  return list;
}
