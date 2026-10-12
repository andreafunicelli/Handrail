import { describe, expect, it } from "vitest";
import { en } from "../i18n/en";
import { settings } from "./settings.svelte";
import { tour, TOUR_STEPS } from "./tour.svelte";
import { ui } from "./ui.svelte";

describe("tour", () => {
  it("walks forward and back, and ends after the last step", () => {
    ui.paletteOpen = true;
    tour.start();
    expect(tour.step).toBe(0);
    expect(ui.paletteOpen).toBe(false);
    tour.back();
    expect(tour.step).toBe(0);
    for (let i = 1; i < TOUR_STEPS.length; i++) {
      tour.next();
      expect(tour.step).toBe(i);
    }
    tour.back();
    expect(tour.step).toBe(TOUR_STEPS.length - 2);
    tour.next();
    tour.next();
    expect(tour.step).toBeNull();
  });

  it("opens what a step points at", () => {
    settings.aiOpen = false;
    tour.go(TOUR_STEPS.findIndex((s) => s.target === "ai"));
    expect(settings.aiOpen).toBe(true);
    ui.rail = "snippets";
    tour.go(TOUR_STEPS.findIndex((s) => s.target === "connections"));
    expect(ui.rail).toBe("connections");
    tour.stop();
    expect(tour.current).toBeNull();
  });

  it("has text for every step", () => {
    for (const s of TOUR_STEPS) {
      expect(en[s.title]).toBeTruthy();
      expect(en[s.body]).toBeTruthy();
    }
  });
});
