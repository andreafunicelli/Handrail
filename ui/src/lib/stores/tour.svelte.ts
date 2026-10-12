import type { MessageKey } from "../i18n";
import { settings } from "./settings.svelte";
import { ui } from "./ui.svelte";

export interface TourStep {
  /** `data-tour` value of the element to point at; none, or not on screen: a centered card. */
  target?: string;
  title: MessageKey;
  body: MessageKey;
  /** Make the target visible first (open a panel, switch the rail). */
  prepare?: () => void;
}

export const TOUR_STEPS: TourStep[] = [
  { title: "tour.welcome.title", body: "tour.welcome.body" },
  { target: "connections", title: "tour.connections.title", body: "tour.connections.body", prepare: () => (ui.rail = "connections") },
  { target: "kinds", title: "tour.files.title", body: "tour.files.body", prepare: () => (ui.rail = "connections") },
  { target: "tabs", title: "tour.tabs.title", body: "tour.tabs.body" },
  { target: "ai", title: "tour.ai.title", body: "tour.ai.body", prepare: () => (settings.aiOpen = true) },
  { target: "snippets", title: "tour.snippets.title", body: "tour.snippets.body" },
  { title: "tour.palette.title", body: "tour.palette.body" },
  { target: "settings", title: "tour.vault.title", body: "tour.vault.body" },
];

/** The guided tour shown once after a vault is created; it can be replayed from the palette. */
class Tour {
  /** Index into [`TOUR_STEPS`], or null when the tour is not running. */
  step = $state<number | null>(null);

  get current(): TourStep | null {
    return this.step == null ? null : TOUR_STEPS[this.step];
  }
  start() {
    ui.paletteOpen = false;
    ui.settingsPage = null;
    this.go(0);
  }
  go(i: number) {
    if (i < 0 || i >= TOUR_STEPS.length) return this.stop();
    TOUR_STEPS[i].prepare?.();
    this.step = i;
  }
  next() {
    if (this.step != null) this.go(this.step + 1);
  }
  back() {
    if (this.step) this.go(this.step - 1);
  }
  stop() {
    this.step = null;
  }
}

export const tour = new Tour();
