import { isAuthError, friendlyError } from './errors';

export type ToastKind = 'info' | 'error' | 'auth';

export interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
}

type Listener = (items: Toast[]) => void;

let seq = 1;
let items: Toast[] = [];
const listeners = new Set<Listener>();
const timers = new Map<number, ReturnType<typeof setTimeout>>();

function emit() {
  const snap = items.slice();
  for (const fn of listeners) fn(snap);
}

export function subscribeToasts(fn: Listener): () => void {
  listeners.add(fn);
  fn(items.slice());
  return () => listeners.delete(fn);
}

export function dismissToast(id: number) {
  const t = timers.get(id);
  if (t) clearTimeout(t);
  timers.delete(id);
  items = items.filter((x) => x.id !== id);
  emit();
}

export function toast(text: string, kind: ToastKind = 'info', holdMs = 4200): number {
  const id = seq++;
  if (kind === 'auth') items = items.filter((x) => x.kind !== 'auth');
  items = [...items, { id, kind, text }].slice(-4);
  emit();
  if (kind !== 'auth' && holdMs > 0) {
    timers.set(id, setTimeout(() => dismissToast(id), holdMs));
  }
  return id;
}

const AUTH: Record<string, string> = {
  like: 'Sign in to like songs',
  dislike: 'Sign in to dislike songs',
  save: 'Sign in to save this to your library',
  playlist: 'Sign in to add songs to a playlist',
  subscribe: 'Sign in to subscribe',
  expired: 'Your session expired. Sign in again.',
  continue: 'Sign in to continue',
};

export type AuthReason = keyof typeof AUTH;

export function authPrompt(reason: AuthReason | string = 'continue'): number {
  const text = AUTH[reason] ?? (reason.startsWith('Sign in') ? reason : `Sign in to ${reason}`);
  return toast(text, 'auth', 0);
}

/** Report a caught error: auth prompt for 401, otherwise a short toast. Never raw HTTP. */
export function fail(e: unknown, fallback: string, authReason?: AuthReason | string): void {
  if (isAuthError(e)) {
    authPrompt(authReason || 'expired');
    return;
  }
  toast(friendlyError(e, fallback), 'error');
}
