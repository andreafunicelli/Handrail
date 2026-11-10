import { describe, expect, it } from "vitest";
import { createNamespaceSql, dropNamespaceSql, dropTableSql, quoteIdent, renameTableSql, truncateTableSql, validName } from "./ddl";

describe("structure SQL", () => {
  it("quotes names per dialect", () => {
    expect(quoteIdent("mysql", "a`b")).toBe("`a``b`");
    expect(quoteIdent("postgres", 'a"b')).toBe('"a""b"');
  });

  it("builds table and namespace statements", () => {
    expect(dropTableSql("mysql", "shop", "orders", false)).toBe("DROP TABLE `shop`.`orders`;");
    expect(dropTableSql("postgres", "public", "v", true)).toBe('DROP VIEW "public"."v";');
    expect(truncateTableSql("postgres", "public", "t")).toBe('TRUNCATE TABLE "public"."t";');
    expect(renameTableSql("mysql", "shop", "a", "b")).toBe("RENAME TABLE `shop`.`a` TO `shop`.`b`;");
    expect(renameTableSql("postgres", "public", "a", "b; DROP TABLE x")).toBe('ALTER TABLE "public"."a" RENAME TO "b; DROP TABLE x";');
    expect(createNamespaceSql("mysql", "new db")).toBe("CREATE DATABASE `new db`;");
    expect(dropNamespaceSql("postgres", "s")).toBe('DROP SCHEMA "s";');
  });

  it("checks names", () => {
    expect(validName(" orders ")).toBe(true);
    expect(validName("  ")).toBe(false);
    expect(validName("a\nb")).toBe(false);
    expect(validName("x".repeat(64))).toBe(false);
  });
});
