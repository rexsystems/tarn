<script lang="ts">
  let { kind = 'home' }: { kind?: 'home' | 'explore' | 'search' | 'artist' | 'album' | 'page' } = $props();
</script>

{#if kind === 'home' || kind === 'explore'}
  <div class="skel" aria-hidden="true" aria-busy="true">
    {#if kind === 'home'}
      <div class="skel-chips">
        {#each { length: 8 } as _, i (i)}<div class="shimmer skel-chip"></div>{/each}
      </div>
    {:else}
      <div class="shimmer skel-h1"></div>
    {/if}
    {#each { length: 2 } as _, s (s)}
      <div class="shimmer skel-title"></div>
      <div class="skel-row">
        {#each { length: 6 } as _, i (i)}
          <div class="skel-card">
            <div class="shimmer skel-cover"></div>
            <div class="shimmer skel-ln"></div>
            <div class="shimmer skel-ln short"></div>
          </div>
        {/each}
      </div>
    {/each}
  </div>
{:else if kind === 'search'}
  <div class="skel" aria-hidden="true" aria-busy="true">
    <div class="shimmer skel-search"></div>
    <div class="skel-tabs">
      {#each { length: 6 } as _, i (i)}<div class="shimmer skel-chip"></div>{/each}
    </div>
    {#each { length: 8 } as _, i (i)}
      <div class="skel-track">
        <div class="shimmer skel-sq"></div>
        <div class="skel-tmeta">
          <div class="shimmer skel-ln"></div>
          <div class="shimmer skel-ln short"></div>
        </div>
      </div>
    {/each}
  </div>
{:else if kind === 'artist'}
  <div class="skel" aria-hidden="true" aria-busy="true">
    <div class="skel-hero">
      <div class="shimmer skel-avatar"></div>
      <div class="skel-hmeta">
        <div class="shimmer skel-h1"></div>
        <div class="shimmer skel-ln"></div>
        <div class="skel-acts">
          <div class="shimmer skel-btn"></div>
          <div class="shimmer skel-btn"></div>
        </div>
      </div>
    </div>
    {#each { length: 6 } as _, i (i)}
      <div class="skel-track">
        <div class="shimmer skel-sq"></div>
        <div class="skel-tmeta">
          <div class="shimmer skel-ln"></div>
          <div class="shimmer skel-ln short"></div>
        </div>
      </div>
    {/each}
  </div>
{:else}
  <!-- album / playlist / generic page -->
  <div class="skel" aria-hidden="true" aria-busy="true">
    <div class="skel-hero">
      <div class="shimmer skel-cover lg"></div>
      <div class="skel-hmeta">
        <div class="shimmer skel-h1"></div>
        <div class="shimmer skel-ln"></div>
        <div class="skel-acts">
          <div class="shimmer skel-btn"></div>
          <div class="shimmer skel-btn"></div>
        </div>
      </div>
    </div>
    {#each { length: 8 } as _, i (i)}
      <div class="skel-track">
        <div class="shimmer skel-sq"></div>
        <div class="skel-tmeta">
          <div class="shimmer skel-ln"></div>
          <div class="shimmer skel-ln short"></div>
        </div>
      </div>
    {/each}
  </div>
{/if}
