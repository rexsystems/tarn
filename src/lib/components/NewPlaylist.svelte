<script lang="ts">
  import { t as tr } from '../i18n.svelte';

  let { onClose, onCreate }: { onClose: () => void; onCreate: (title: string) => void | Promise<void> } = $props();
  let name = $state('');
  let busy = $state(false);

  async function submit() {
    const t = name.trim();
    if (!t || busy) return;
    busy = true;
    try { await onCreate(t); } finally { busy = false; }
    onClose();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="np-scrim" onclick={onClose}>
  <div class="np-box" role="dialog" aria-modal="true" onclick={(e) => e.stopPropagation()}>
    <h3>{tr('New playlist')}</h3>
    <!-- svelte-ignore a11y_autofocus -->
    <input
      class="np-in"
      type="text"
      autofocus
      placeholder={tr('Playlist name')}
      bind:value={name}
      onkeydown={(e) => { if (e.key === 'Enter') void submit(); else if (e.key === 'Escape') onClose(); }}
    />
    <div class="np-row">
      <button class="btn ghost" type="button" onclick={onClose}>{tr('Cancel')}</button>
      <button class="btn" type="button" disabled={!name.trim() || busy} onclick={submit}>{tr('Create')}</button>
    </div>
  </div>
</div>

<style>
  .np-scrim { position: fixed; inset: 0; z-index: 400; background: rgba(0, 0, 0, 0.5); display: grid; place-items: center; }
  .np-box { width: min(380px, 90vw); background: var(--bg-elev, #1c1c1c); color: var(--text); border-radius: 14px; padding: 20px; box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5); }
  h3 { margin: 0 0 12px; font-size: 16px; }
  .np-in { width: 100%; box-sizing: border-box; padding: 10px 12px; border-radius: 8px; border: 1px solid var(--border, #444); background: var(--bg-active, #2a2a2a); color: inherit; font: inherit; }
  .np-row { display: flex; justify-content: flex-end; gap: 8px; margin-top: 14px; }
</style>
