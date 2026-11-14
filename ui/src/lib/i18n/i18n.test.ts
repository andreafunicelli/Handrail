import { describe, expect, it as test } from "vitest";
import { en } from "./en";
import { it } from "./it";
import { locale, t, tn } from "./index";

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

  test("plural keys come in _one/_other pairs and pick the form for the count", () => {
    for (const cat of [en, it] as Record<string, string>[])
      for (const k of Object.keys(cat)) if (k.endsWith("_one")) expect(k.replace(/_one$/, "_other") in cat, k).toBe(true);
    expect(tn("db.rows", 1)).toBe("1 row");
    expect(tn("db.rows", 2500)).toBe("2,500 rows");
    locale.set("it");
    expect(tn("db.rows", 1)).toBe("1 riga");
    expect(tn("db.rows", 0)).toBe("0 righe");
    expect(tn("db.rows", 25000)).toBe("25.000 righe");
    locale.set("en");
  });
});
