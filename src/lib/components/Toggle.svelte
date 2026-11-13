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
  function onKey(e: KeyboardEvent) {
    if (e.key === ' ' || e.key === 'Enter') {
      e.preventDefault();
      toggle();
    }
  }
</script>

<button
  class="tog {checked ? 'on' : ''}"
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  onclick={toggle}
  onkeydown={onKey}
>
  <span class="tog-knob"></span>
</button>
