<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../i18n";
  import { tour, TOUR_STEPS } from "../../stores/tour.svelte";

  /** The guided tour: a spotlight on one part of the window and a card that explains it. */

  const CARD_W = 340;
  const GAP = 14;
  const PAD = 6;
  const mod = typeof navigator !== "undefined" && /Mac|iP/.test(navigator.platform) ? "⌘" : "Ctrl+";

  const step = $derived(tour.current!);
  const last = $derived(tour.step === TOUR_STEPS.length - 1);
  let rect = $state<DOMRect | null>(null);
  let card = $state<HTMLDivElement>();
  let cardH = $state(200);
  let next = $state<HTMLButtonElement>();

  // Follow the target while it renders, resizes or moves (a panel opening, the window resizing).
  onMount(() => {
    let raf = 0;
    const tick = () => {
      const el = step.target ? document.querySelector<HTMLElement>(`[data-tour="${step.target}"]`) : null;
      const r = el?.getBoundingClientRect();
      const shown = r && r.width > 0 && r.height > 0 ? r : null;
      if (shown?.x !== rect?.x || shown?.y !== rect?.y || shown?.width !== rect?.width || shown?.height !== rect?.height) rect = shown ?? null;
      if (card) cardH = card.offsetHeight;
      raf = requestAnimationFrame(tick);
    };
    tick();
    return () => cancelAnimationFrame(raf);
  });

  $effect(() => {
    void tour.step;
    next?.focus();
  });

  /** Beside the target where there is room (right, left, below, above), else centered. */
  const pos = $derived.by(() => {
    const vw = innerWidth;
    const vh = innerHeight;
    const w = Math.min(CARD_W, vw - 32);
    const clampX = (x: number) => Math.max(16, Math.min(x, vw - w - 16));
    const clampY = (y: number) => Math.max(16, Math.min(y, vh - cardH - 16));
    if (!rect) return { left: (vw - w) / 2, top: Math.max(16, (vh - cardH) / 2), w };
    if (rect.right + GAP + w <= vw - 16) return { left: rect.right + GAP, top: clampY(rect.top), w };
    if (rect.left - GAP - w >= 16) return { left: rect.left - GAP - w, top: clampY(rect.top), w };
    if (rect.bottom + GAP + cardH <= vh - 16) return { left: clampX(rect.left), top: rect.bottom + GAP, w };
    if (rect.top - GAP - cardH >= 16) return { left: clampX(rect.left), top: rect.top - GAP - cardH, w };
    return { left: (vw - w) / 2, top: Math.max(16, (vh - cardH) / 2), w };
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") tour.stop();
    else if (e.key === "ArrowRight") tour.next();
    else if (e.key === "ArrowLeft") tour.back();
    else return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<svelte:window onkeydowncapture={onkeydown} />

<div class="tour" class:dim={!rect}>
  {#if rect}
    <div
      class="spot"
      style:left="{rect.left - PAD}px"
      style:top="{rect.top - PAD}px"
      style:width="{rect.width + PAD * 2}px"
      style:height="{rect.height + PAD * 2}px"
    ></div>
  {/if}
  <div
    class="card"
    bind:this={card}
    role="dialog"
    aria-modal="true"
    aria-labelledby="tour-title"
    aria-describedby="tour-body"
    style:left="{pos.left}px"
    style:top="{pos.top}px"
    style:width="{pos.w}px"
  >
    <span class="count">{t("tour.count", { n: (tour.step ?? 0) + 1, total: TOUR_STEPS.length })}</span>
    <h2 id="tour-title">{t(step.title)}</h2>
    <p id="tour-body">{t(step.body, { mod })}</p>
    <div class="dots" aria-hidden="true">
      {#each TOUR_STEPS as _, i (i)}<span class:on={i === tour.step}></span>{/each}
    </div>
    <div class="actions">
      {#if !last}<button class="btn ghost sm skip" onclick={() => tour.stop()}>{t("tour.skip")}</button>{/if}
      <span class="grow"></span>
      {#if tour.step}<button class="btn sm" onclick={() => tour.back()}>{t("tour.back")}</button>{/if}
      <button class="btn sm primary" bind:this={next} onclick={() => tour.next()}>{last ? t("tour.done") : t("tour.next")}</button>
    </div>
  </div>
</div>

<style>
  .tour {
    position: fixed;
    inset: 0;
    z-index: 200;
  }
  .tour.dim {
    background: rgba(0, 0, 0, 0.55);
  }
  .spot {
    position: fixed;
    border-radius: var(--r-lg);
    box-shadow:
      0 0 0 2px var(--accent),
      0 0 0 9999px rgba(0, 0, 0, 0.55);
    transition:
      left 0.25s var(--ease),
      top 0.25s var(--ease),
      width 0.25s var(--ease),
      height 0.25s var(--ease);
    pointer-events: none;
  }
  .card {
    position: fixed;
    display: grid;
    gap: 10px;
    padding: 18px 18px 14px;
    border-radius: var(--r-lg);
    background: var(--panel);
    box-shadow: var(--shadow-float);
    transition:
      left 0.25s var(--ease),
      top 0.25s var(--ease);
  }
  .count {
    color: var(--faint);
    font-size: var(--fs-small);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  p {
    margin: 0;
    color: var(--text-2);
    line-height: 1.5;
  }
  .dots {
    display: flex;
    gap: 5px;
  }
  .dots span {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--border-strong);
  }
  .dots span.on {
    background: var(--accent);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
  }
  .grow {
    flex: 1;
  }
  .skip {
    color: var(--muted);
  }
  @media (prefers-reduced-motion: reduce) {
    .spot,
    .card {
      transition: none;
    }
  }
</style>
