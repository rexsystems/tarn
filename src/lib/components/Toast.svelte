<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../api';
  import { subscribeToasts, dismissToast, type Toast } from '../toast';
  import Hi from './Hi.svelte';
  import { Login03Icon, Cancel01Icon } from '../icons';

  let items = $state<Toast[]>([]);

  onMount(() => subscribeToasts((t) => (items = t)));

  async function signIn(id: number) {
    dismissToast(id);
    try {
      await api.openLogin();
    } catch {
      /* the login window handles its own errors */
    }
  }
</script>

{#if items.length}
  <div class="toasts" role="status" aria-live="polite">
    {#each items as t (t.id)}
      <div class="toast {t.kind}">
        <div class="toast-text">{t.text}</div>
        {#if t.kind === 'auth'}
          <button class="btn" type="button" onclick={() => signIn(t.id)}>
            <Hi icon={Login03Icon} size={16} /> Sign in
          </button>
        {/if}
        <button class="toast-x" type="button" aria-label="Dismiss" onclick={() => dismissToast(t.id)}>
          <Hi icon={Cancel01Icon} size={14} />
        </button>
      </div>
    {/each}
  </div>
{/if}
