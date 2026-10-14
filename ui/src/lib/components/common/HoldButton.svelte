<script lang="ts">
  import { onDestroy, type Snippet } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";

  /**
   * A button that acts only after it is held down: pointer, Space or Enter. Letting go early,
   * dragging away, Escape, losing focus or hiding the window drains the fill back and nothing
   * happens. Behaviour ported from Interior's hold-to-confirm (github.com/ddoemonn/interior, MIT).
   */
  let {
    onconfirm,
    children,
    confirmLabel = t("hold.done"),
    duration = 1500,
    variant = "danger",
    size = "",
    disabled = false,
    title,
  }: {
    onconfirm: () => void;
    children: Snippet;
    confirmLabel?: string;
    duration?: number;
    variant?: "danger" | "primary";
    size?: "" | "sm" | "lg";
    disabled?: boolean;
    title?: string;
  } = $props();

  /** Draining back is faster than filling, so a second try starts from where the first left off. */
  const RELEASE_RATE = 2.5;
  /** Pointer travel (px) that counts as dragging away. */
  const MOVE_TOLERANCE = 10;

  let phase = $state<"idle" | "holding" | "releasing" | "done">("idle");
  let progress = $state(0);
  let held = false;
  let elapsed = 0;
  let last = 0;
  let raf = 0;
  let origin: { x: number; y: number } | null = null;
  let el: HTMLButtonElement;
  const hintId = `hold-${Math.random().toString(36).slice(2)}`;
  const seconds = $derived(Math.round(duration / 100) / 10);

  function frame(now: number) {
    const dt = Math.min(64, now - last);
    last = now;
    elapsed += held ? dt : -dt * RELEASE_RATE;
    if (elapsed >= duration) {
      raf = 0;
      progress = 1;
      held = false;
      origin = null;
      phase = "done";
      onconfirm();
      return;
    }
    if (elapsed <= 0) {
      raf = 0;
      elapsed = 0;
      progress = 0;
      phase = "idle";
      return;
    }
    progress = elapsed / duration;
    raf = requestAnimationFrame(frame);
  }

  export function focus() {
    el.focus();
  }

  /** Start (or resume) holding. A parent forwarding a held key focuses this button first, so
   *  the key's release lands here. */
  export function begin(point?: { x: number; y: number }) {
    if (disabled || phase === "done" || phase === "holding") return;
    origin = point ?? null;
    held = true;
    phase = "holding";
    if (raf) return;
    last = performance.now();
    raf = requestAnimationFrame(frame);
  }

  function release() {
    if (phase !== "holding") return;
    held = false;
    origin = null;
    phase = "releasing";
  }

  /** Back to rest, e.g. after the action failed. */
  export function reset() {
    cancelAnimationFrame(raf);
    raf = 0;
    held = false;
    elapsed = 0;
    origin = null;
    progress = 0;
    phase = "idle";
  }

  function pointerdown(e: PointerEvent) {
    if (e.pointerType === "mouse" && e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    begin({ x: e.clientX, y: e.clientY });
  }
  function pointermove(e: PointerEvent) {
    if (phase === "holding" && origin && Math.hypot(e.clientX - origin.x, e.clientY - origin.y) > MOVE_TOLERANCE) release();
  }
  function keydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (phase === "holding" || phase === "releasing") {
        e.preventDefault();
        e.stopPropagation();
        reset();
      }
      return;
    }
    if (e.key === " " || e.key === "Enter") {
      // Swallow repeats and keep the key from reaching outer shortcuts (forms, cards).
      e.preventDefault();
      e.stopPropagation();
      if (!e.repeat) begin();
    }
  }
  function keyup(e: KeyboardEvent) {
    if (e.key === " " || e.key === "Enter") release();
  }

  $effect(() => {
    const visibility = () => document.hidden && release();
    window.addEventListener("blur", release);
    document.addEventListener("visibilitychange", visibility);
    return () => {
      window.removeEventListener("blur", release);
      document.removeEventListener("visibilitychange", visibility);
    };
  });
  onDestroy(() => cancelAnimationFrame(raf));
</script>

<button
  bind:this={el}
  type="button"
  class="btn hold {variant} {size}"
  class:holding={phase === "holding"}
  class:off={disabled}
  aria-disabled={disabled || phase === "done"}
  aria-describedby={hintId}
  {title}
  onpointerdown={pointerdown}
  onpointermove={pointermove}
  onpointerup={release}
  onpointercancel={release}
  onpointerleave={release}
  onkeydown={keydown}
  onkeyup={keyup}
  onblur={release}
  onclick={(e) => e.preventDefault()}
  oncontextmenu={(e) => e.preventDefault()}
>
  <!-- Both faces share one grid cell, so the button never changes width between them. -->
  <span class="faces">
    <span class="face" class:off={phase === "done"}>{@render children()}</span>
    <span class="face" class:off={phase !== "done"} aria-hidden={phase !== "done"}><Icon name="check" size={12} />{confirmLabel}</span>
  </span>
  <span class="fill" aria-hidden="true" style:clip-path="inset(0 {(1 - progress) * 100}% 0 0)">
    <span class="faces">
      <span class="face" class:off={phase === "done"}>{@render children()}</span>
      <span class="face" class:off={phase !== "done"}><Icon name="check" size={12} />{confirmLabel}</span>
    </span>
  </span>
  <span id={hintId} class="visually-hidden">{t("hold.hint", { seconds })}</span>
  <span class="visually-hidden" role="status" aria-live="polite">{phase === "done" ? confirmLabel : ""}</span>
</button>

<style>
  .hold {
    position: relative;
    isolation: isolate;
    overflow: hidden;
    user-select: none;
    -webkit-user-select: none;
    touch-action: manipulation;
  }
  /* The resting face is quiet; the colour arrives with the fill, so the hold itself is the
     decision. */
  .hold.danger,
  .hold.primary {
    background: var(--raised);
    border-color: var(--border-strong);
    color: var(--text);
    font-weight: 550;
  }
  .hold.danger {
    border-color: color-mix(in srgb, var(--danger) 55%, var(--border-strong));
    color: var(--danger);
  }
  .hold.primary {
    border-color: var(--accent-line);
    color: var(--accent-text);
  }
  .hold[aria-disabled="true"] {
    cursor: default;
  }
  .hold.off {
    opacity: 0.45;
  }
  .faces {
    display: grid;
  }
  .face {
    grid-area: 1 / 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    white-space: nowrap;
    transition: opacity var(--t) var(--ease);
  }
  .face.off {
    opacity: 0;
  }
  .fill {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    padding: inherit;
    pointer-events: none;
  }
  .danger .fill {
    background: var(--danger);
    color: #fff;
  }
  .primary .fill {
    background: var(--accent);
    color: var(--on-accent);
  }
</style>
