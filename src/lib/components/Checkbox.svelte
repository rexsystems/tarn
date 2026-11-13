<script lang="ts">
  let {
    checked = $bindable(false),
    onchange,
    disabled = false,
    label,
  }: {
    checked: boolean;
    onchange?: (v: boolean) => void;
    disabled?: boolean;
    label?: string;
  } = $props();

  function toggle() {
    if (disabled) return;
    checked = !checked;
    onchange?.(checked);
  }
</script>

<button
  class="chk {checked ? 'on' : ''}"
  type="button"
  role="checkbox"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  onclick={toggle}
>
  <span class="chk-box" aria-hidden="true">
    <svg width="12" height="12" viewBox="0 0 12 12">
      <path d="M2 6.2l2.6 2.6L10 3.2" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>
  </span>
  {#if label}<span class="chk-lb">{label}</span>{/if}
</button>
