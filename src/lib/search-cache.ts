import { api } from './api';
import type { SearchResults } from './types';

const TTL = 5 * 60 * 1000;

interface Hit {
  at: number;
  suggestions: string[];
  results: SearchResults | null;
}

const mem = new Map<string, Hit>();

export function cacheKey(q: string): string {
  return q.trim().toLowerCase();
}

export function peekSearch(q: string): Hit | null {
  const k = cacheKey(q);
  if (!k) return null;
  const hit = mem.get(k);
  if (!hit || Date.now() - hit.at > TTL) return null;
  return hit;
}

export async function liveSearch(q: string): Promise<Hit> {
  const k = cacheKey(q);
  const cached = peekSearch(q);
  if (cached) return cached;
  const [suggestions, results] = await Promise.all([
    api.suggestions(q).catch(() => [] as string[]),
    api.searchAll(q).catch(() => null),
  ]);
  const hit: Hit = { at: Date.now(), suggestions, results };
  if (k) mem.set(k, hit);
  // Bound the cache so long sessions don't grow forever.
  if (mem.size > 80) {
    const first = mem.keys().next().value;
    if (first) mem.delete(first);
  }
  return hit;
}
