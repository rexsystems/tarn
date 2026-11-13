<script lang="ts">
  let {
    value = 0,
    max = 1,
    disabled = false,
    live = false,
    wheel = false,
    showTip = false,
    format = (v: number) => String(v),
    buffered = 0,
    oninput,
    onchange,
    onpreview,
    class: cls = '',
  }: {
    value?: number;
    max?: number;
    disabled?: boolean;
    live?: boolean;
    wheel?: boolean;
    showTip?: boolean;
    format?: (v: number) => string;
    /** Seconds (or same unit as max) already downloaded ahead. */
    buffered?: number;
    oninput?: (v: number) => void;
    onchange?: (v: number) => void;
    onpreview?: (v: number | null) => void;
    class?: string;
  } = $props();

  let root: HTMLDivElement | undefined = $state();
  let dragging = $state(false);
  let hovering = $state(false);
  let preview = $state<number | null>(null);
  let hoverPct = $state(0);

  let shown = $derived(preview ?? value);
  let fillPct = $derived.by(() => {
    if (!max || max <= 0 || !isFinite(max)) return 0;
    return Math.max(0, Math.min(100, (shown / max) * 100));
  });
  let tipPct = $derived(dragging || preview != null ? fillPct : hoverPct);
  let tipVal = $derived(preview ?? (hovering && max > 0 ? (hoverPct / 100) * max : shown));
  let bufPct = $derived.by(() => {
    if (!max || max <= 0 || !isFinite(max) || !buffered) return 0;
    return Math.max(0, Math.min(100, (buffered / max) * 100));
  });

  function atX(clientX: number): number {
    const r = root!.getBoundingClientRect();
    const t = r.width > 0 ? (clientX - r.left) / r.width : 0;
    const ratio = Math.max(0, Math.min(1, t));
    const m = max > 0 && isFinite(max) ? max : 1;
    return ratio * m;
  }

  function apply(v: number, fireLive: boolean) {
    preview = v;
    onpreview?.(v);
    if (fireLive) oninput?.(v);
  }

  function down(e: PointerEvent) {
    if (disabled || !root || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    dragging = true;
    try { root.setPointerCapture(e.pointerId); } catch { /* WebKitGTK may lack capture */ }
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', up);
    apply(atX(e.clientX), live);
  }

  function move(e: PointerEvent) {
    if (!dragging || !root) return;
    apply(atX(e.clientX), live);
  }

  function up(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    window.removeEventListener('pointercancel', up);
    const v = root ? atX(e.clientX) : (preview ?? value);
    hoverPct = max > 0 ? (v / max) * 100 : 0;
    if (live) oninput?.(v);
    onchange?.(v);
    preview = null;
    onpreview?.(null);
    if (root) {
      try { root.releasePointerCapture(e.pointerId); } catch { /* ignore */ }
    }
  }

  function onEnter() { hovering = true; }
  function onLeave() {
    hovering = false;
    if (!dragging) {
      preview = null;
      onpreview?.(null);
    }
  }
  function onHoverMove(e: PointerEvent) {
    if (dragging || !root || disabled || !max) return;
    const v = atX(e.clientX);
    hoverPct = max > 0 ? (v / max) * 100 : 0;
  }

  function onWheel(e: WheelEvent) {
    if (!wheel || disabled) return;
    e.preventDefault();
    const step = (max || 1) * 0.05;
    const next = Math.max(0, Math.min(max || 1, value + (e.deltaY > 0 ? -step : step)));
    oninput?.(next);
    onchange?.(next);
  }

  function onKey(e: KeyboardEvent) {
    if (disabled || !max) return;
    let next: number | null = null;
    const step = max * (e.shiftKey ? 0.1 : 0.02);
    if (e.key === 'ArrowRight' || e.key === 'ArrowUp') next = Math.min(max, value + step);
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') next = Math.max(0, value - step);
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = max;
    if (next == null) return;
    e.preventDefault();
    if (live) oninput?.(next);
    onchange?.(next);
  }
</script>

<div
  bind:this={root}
  class="sbar {cls}"
  class:dragging
  class:hovering
  class:disabled
  role="slider"
  aria-valuemin={0}
  aria-valuemax={max}
  aria-valuenow={shown}
  aria-disabled={disabled}
  tabindex={disabled ? -1 : 0}
  onpointerdown={down}
  onpointerenter={onEnter}
  onpointerleave={onLeave}
  onpointermove={onHoverMove}
  onwheel={onWheel}
  onkeydown={onKey}
>
  <div class="sbar-track">
    {#if bufPct > 0}
      <div class="sbar-buf" style="width:{bufPct}%"></div>
    {/if}
    <div class="sbar-fill" style="width:{fillPct}%"></div>
  </div>
  <div class="sbar-thumb" style="left:{fillPct}%"></div>
  {#if showTip && (hovering || dragging) && max > 0}
    <div class="sbar-tip" style="left:{tipPct}%">{format(tipVal)}</div>
  {/if}
</div>
