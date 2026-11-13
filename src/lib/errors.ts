/** Map raw backend / network errors to short, user-facing copy. */

export function isAuthError(e: unknown): boolean {
  const s = String(e ?? '').toLowerCase();
  return /401|unauthorized|not logged|no login|unauthenticated|sign.?in required|auth(entication)? (failed|required|expired)/.test(s);
}

export function friendlyError(
  e: unknown,
  fallback = 'Something went wrong. Try again.',
): string {
  const s = String(e ?? '').trim();
  const low = s.toLowerCase();
  if (!s) return fallback;
  if (isAuthError(e)) return 'Sign in to continue';
  if (/403|forbidden/.test(low)) {
    if (/audio|download|stream|googlevideo/.test(low)) return "Can't play this track";
    return "You don't have access to this.";
  }
  if (/404|not found/.test(low)) return "We couldn't find that.";
  if (/network|failed to fetch|fetch failed|connection|timed out|timeout|dns|offline/.test(low)) {
    return "Can't reach YouTube right now.";
  }
  if (/innertube|http\s*\d+|status(?:\s*code)?\s*\d+|reqwest|rpc |error code|os error/.test(low)) {
    return fallback;
  }
  if (s.length < 80 && !/https?:|error:|rust|panic|stack|at \//i.test(s)) return s;
  return fallback;
}
