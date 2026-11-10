import { describe, expect, it as test } from "vitest";
import { en } from "./en";
import { it } from "./it";
import { locale, t } from "./index";

const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

describe("i18n", () => {
  test("Italian has only known keys, with the same placeholders", () => {
    for (const [k, v] of Object.entries(it)) {
      expect(k in en, `unknown key ${k}`).toBe(true);
      expect(placeholders(v as string), k).toEqual(placeholders(en[k as keyof typeof en]));
    }
  });

  test("switching language changes text; a missing key falls back to English", () => {
    locale.set("it");
    expect(t("common.cancel")).toBe(it["common.cancel"]);
    locale.set("en");
    expect(t("common.cancel")).toBe(en["common.cancel"]);
    expect(t("app.name")).toBe("Handrail");
    expect(t("sidebar.noResults", { q: "x" })).toBe(en["sidebar.noResults"].replace("{q}", "x"));
  });
});
