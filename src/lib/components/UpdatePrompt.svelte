<script lang="ts">
  import { onMount } from 'svelte';
  import { toast } from '../toast';

  let {
    request = $bindable(0),
  }: {
    /** Increment to force a manual check (Settings). */
    request?: number;
  } = $props();

  let open = $state(false);
  let version = $state('');
  let notes = $state('');
  let date = $state('');
  let progress = $state<number | null>(null);
  let installing = $state(false);
  let err = $state('');
  let update: Awaited<ReturnType<typeof import('@tauri-apps/plugin-updater').check>> = null;
  let lastRequest = 0;

  async function load() {
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      return await check();
    } catch {
      return null;
    }
  }

  async function run(manual: boolean) {
    err = '';
    const u = await load();
    update = u;
    if (!u) {
      if (manual) toast("You're on the latest version");
      return;
    }
    version = u.version;
    notes = (u.body || '').trim();
    date = u.date ? String(u.date) : '';
    progress = null;
    installing = false;
    open = true;
  }

  // Running total for Progress events
  let received = 0;
  let total = 0;
  async function installTracked() {
    if (!update || installing) return;
    installing = true;
    err = '';
    progress = 0;
    received = 0;
    total = 0;
    try {
      await update.downloadAndInstall((ev) => {
        if (ev.event === 'Started') {
          total = Number((ev.data as { contentLength?: number }).contentLength ?? 0);
          received = 0;
          progress = 0;
        } else if (ev.event === 'Progress') {
          received += Number((ev.data as { chunkLength?: number }).chunkLength ?? 0);
          progress = total > 0 ? Math.min(1, received / total) : 0;
        } else if (ev.event === 'Finished') {
          progress = 1;
        }
      });
      const { relaunch } = await import('@tauri-apps/plugin-process');
      await relaunch();
    } catch (e) {
      installing = false;
      progress = null;
      err = "Couldn't install the update. Try again, or download it from the releases page.";
      console.error(e);
    }
  }

  function later() {
    if (installing) return;
    open = false;
  }

  onMount(() => {
    const t = setTimeout(() => run(false), 4000);
    return () => clearTimeout(t);
  });

  $effect(() => {
    const n = request;
    if (n && n !== lastRequest) {
      lastRequest = n;
      run(true);
    }
  });
</script>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="upd-back" onmousedown={(e) => { if (e.target === e.currentTarget && !installing) later(); }}>
    <div class="upd" role="dialog" aria-labelledby="upd-title" aria-modal="true">
      <div class="upd-k">Update available</div>
      <h2 id="upd-title" class="upd-title">Tarn {version} is ready</h2>
      {#if notes}
        <div class="upd-notes">{notes}</div>
      {/if}
      {#if progress != null}
        <div class="upd-bar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress * 100)} role="progressbar">
          <span style="width:{Math.round(progress * 100)}%"></span>
        </div>
        <div class="upd-prog">{progress >= 1 ? 'Restarting…' : `Downloading ${Math.round(progress * 100)}%`}</div>
      {/if}
      {#if err}
        <div class="upd-err">{err}</div>
      {/if}
      <div class="upd-actions">
        <button type="button" class="upd-later" disabled={installing} onclick={later}>Later</button>
        <button type="button" class="upd-go" disabled={installing} onclick={installTracked}>
          {installing ? 'Installing…' : 'Install and restart'}
        </button>
      </div>
    </div>
  </div>
{/if}
