<script lang="ts">
  import { t } from "../../i18n";
  /** Vertical drag handle for a sidebar. `edge` says which side of the panel the handle sits on. */
  let {
    value,
    min,
    max,
    edge = "right",
    onchange,
  }: { value: number; min: number; max: number; edge?: "left" | "right"; onchange: (v: number) => void } = $props();

  let dragging = $state(false);
  const clamp = (v: number) => Math.round(Math.min(max, Math.max(min, v)));

  function down(e: PointerEvent) {
    e.preventDefault();
    const start = e.clientX;
    const startVal = value;
    dragging = true;
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const d = ev.clientX - start;
      onchange(clamp(startVal + (edge === "right" ? d : -d)));
    };
    const up = () => {
      dragging = false;
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  }
  function key(e: KeyboardEvent) {
    const step = e.shiftKey ? 40 : 12;
    const dir = edge === "right" ? 1 : -1;
    if (e.key === "ArrowLeft") onchange(clamp(value - step * dir));
    if (e.key === "ArrowRight") onchange(clamp(value + step * dir));
  }
</script>

<div
  class="resizer"
  class:dragging
  role="separator"
  aria-orientation="vertical"
  aria-valuenow={value}
  aria-valuemin={min}
  aria-valuemax={max}
  aria-label={t("common.resize")}
  tabindex="0"
  onpointerdown={down}
  onkeydown={key}
></div>

<style>
  .resizer {
    position: relative;
    width: 1px;
    flex: none;
    background: var(--border);
    cursor: col-resize;
    z-index: 5;
    touch-action: none;
  }
  .resizer::after {
    content: "";
    position: absolute;
    inset: 0 -4px;
  }
  .resizer:hover,
  .resizer.dragging,
  .resizer:focus-visible {
    background: var(--accent-line);
    outline: none;
  }
</style>
