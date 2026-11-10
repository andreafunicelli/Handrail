import { describe, expect, it } from "vitest";
import {
  alterTableSql,
  createNamespaceSql,
  createTableSql,
  defaultAsSql,
  defProblem,
  dropNamespaceSql,
  dropTableSql,
  quoteIdent,
  renameTableSql,
  truncateTableSql,
  validName,
  type ColDef,
  type TableDef,
} from "./ddl";

describe("structure SQL", () => {
  it("quotes names per dialect", () => {
    expect(quoteIdent("mysql", "a`b")).toBe("`a``b`");
    expect(quoteIdent("postgres", 'a"b')).toBe('"a""b"');
  });

  it("builds table and namespace statements", () => {
    expect(dropTableSql("mysql", "shop", "orders", "table")).toBe("DROP TABLE `shop`.`orders`;");
    expect(dropTableSql("postgres", "public", "v", "view")).toBe('DROP VIEW "public"."v";');
    expect(dropTableSql("postgres", "public", "m", "materialized")).toBe('DROP MATERIALIZED VIEW "public"."m";');
    expect(renameTableSql("postgres", "public", "m", "n", "materialized")).toBe('ALTER MATERIALIZED VIEW "public"."m" RENAME TO "n";');
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

  const col = (name: string, type: string, o: Partial<ColDef> = {}): ColDef => ({ orig: name, name, type, nullable: true, default: null, comment: null, extra: null, ...o });
  const base = (): TableDef => ({
    columns: [col("id", "int", { nullable: false, extra: "auto_increment" }), col("name", "varchar(40)", { default: "'x'" }), col("old", "text")],
    indexes: [
      { orig: "PRIMARY", name: "PRIMARY", columns: ["id"], unique: true, primary: true },
      { orig: "ix_name", name: "ix_name", columns: ["name"], unique: false, primary: false },
    ],
    fks: [{ orig: "fk_a", name: "fk_a", columns: ["id"], refNs: "s", refTable: "p", refColumns: ["id"], onUpdate: "NO ACTION", onDelete: "CASCADE" }],
  });

  it("alters a MySQL table: drops first, then column changes, then additions", () => {
    const after = base();
    after.fks = [];
    after.indexes = [after.indexes[0], { orig: null, name: "ux_title", columns: ["title"], unique: true, primary: false }];
    after.columns = [
      after.columns[0],
      { ...after.columns[1], name: "title", type: "varchar(80)", nullable: false, comment: "it's" },
      col("", "", { orig: null, name: "price", type: "decimal(10,2)", default: "0" }),
    ];
    expect(alterTableSql("mysql", "s", "t", base(), after)).toEqual([
      "ALTER TABLE `s`.`t` DROP FOREIGN KEY `fk_a`;",
      "DROP INDEX `ix_name` ON `s`.`t`;",
      "ALTER TABLE `s`.`t` DROP COLUMN `old`;",
      "ALTER TABLE `s`.`t` CHANGE COLUMN `name` `title` varchar(80) NOT NULL DEFAULT 'x' COMMENT 'it''s';",
      "ALTER TABLE `s`.`t` ADD COLUMN `price` decimal(10,2) NULL DEFAULT 0;",
      "CREATE UNIQUE INDEX `ux_title` ON `s`.`t` (`title`);",
    ]);
    // Nothing changed: nothing to run. The auto-increment column keeps AUTO_INCREMENT when changed.
    expect(alterTableSql("mysql", "s", "t", base(), base())).toEqual([]);
    const ai = base();
    ai.columns[0] = { ...ai.columns[0], type: "bigint" };
    expect(alterTableSql("mysql", "s", "t", base(), ai)).toEqual(["ALTER TABLE `s`.`t` CHANGE COLUMN `id` `id` bigint NOT NULL AUTO_INCREMENT;"]);
    const coll = base();
    coll.columns[1] = { ...coll.columns[1], collation: "utf8mb4_bin" };
    const coll2 = structuredClone(coll);
    coll2.columns[1].type = "varchar(60)";
    expect(alterTableSql("mysql", "s", "t", coll, coll2)).toEqual(["ALTER TABLE `s`.`t` CHANGE COLUMN `name` `name` varchar(60) COLLATE utf8mb4_bin NULL DEFAULT 'x';"]);
  });

  it("alters a PostgreSQL table one property at a time", () => {
    const after = base();
    after.columns[1] = { ...after.columns[1], name: "title", type: "text", nullable: false, default: null, comment: "c" };
    after.indexes = after.indexes.slice(1);
    after.fks.push({ orig: null, name: "fk_b", columns: ["title"], refNs: "s", refTable: "q", refColumns: ["t"], onUpdate: "CASCADE", onDelete: "SET NULL" });
    expect(alterTableSql("postgres", "s", "t", base(), after)).toEqual([
      'ALTER TABLE "s"."t" DROP CONSTRAINT "PRIMARY";',
      'ALTER TABLE "s"."t" RENAME COLUMN "name" TO "title";',
      'ALTER TABLE "s"."t" ALTER COLUMN "title" TYPE text USING "title"::text;',
      'ALTER TABLE "s"."t" ALTER COLUMN "title" SET NOT NULL;',
      'ALTER TABLE "s"."t" ALTER COLUMN "title" DROP DEFAULT;',
      `COMMENT ON COLUMN "s"."t"."title" IS 'c';`,
      'ALTER TABLE "s"."t" ADD CONSTRAINT "fk_b" FOREIGN KEY ("title") REFERENCES "s"."q" ("t") ON UPDATE CASCADE ON DELETE SET NULL;',
    ]);
  });

  it("creates tables", () => {
    const t: TableDef = {
      columns: [
        col("id", "int", { orig: null, nullable: false, extra: "GENERATED BY DEFAULT AS IDENTITY" }),
        col("note", "text", { orig: null, comment: "free text" }),
      ],
      indexes: [{ orig: null, name: "", columns: ["id"], unique: true, primary: true }],
      fks: [],
    };
    expect(createTableSql("postgres", "public", "n", t)).toBe(
      'CREATE TABLE "public"."n" (\n  "id" int GENERATED BY DEFAULT AS IDENTITY,\n  "note" text,\n  PRIMARY KEY ("id")\n);\n' +
        `COMMENT ON COLUMN "public"."n"."note" IS 'free text';`,
    );
    const m: TableDef = { ...t, columns: [col("id", "int", { orig: null, nullable: false, extra: "auto_increment" })] };
    expect(createTableSql("mysql", "shop", "n", m)).toBe("CREATE TABLE `shop`.`n` (\n  `id` int NOT NULL AUTO_INCREMENT,\n  PRIMARY KEY (`id`)\n);");
  });

  it("checks a definition before building SQL", () => {
    expect(defProblem({ columns: [], indexes: [], fks: [] })).toBe("noColumns");
    expect(defProblem({ columns: [col("a", "")], indexes: [], fks: [] })).toBe("columnIncomplete");
    expect(defProblem({ columns: [col("a", "int"), col("A", "int")], indexes: [], fks: [] })).toBe("columnTwice");
    expect(defProblem({ columns: [col("a", "int")], indexes: [{ orig: null, name: "i", columns: ["b"], unique: false, primary: false }], fks: [] })).toBe("indexIncomplete");
    expect(defProblem(base())).toBeNull();
  });

  it("turns reported defaults into SQL", () => {
    expect(defaultAsSql("mysql", false, "abc", "", "varchar(10)")).toBe("'abc'");
    expect(defaultAsSql("mysql", false, "0", "", "int")).toBe("0");
    expect(defaultAsSql("mysql", false, "CURRENT_TIMESTAMP(3)", "", "datetime(3)")).toBe("CURRENT_TIMESTAMP(3)");
    expect(defaultAsSql("mysql", false, "CURRENT_TIMESTAMP", "DEFAULT_GENERATED", "datetime")).toBe("CURRENT_TIMESTAMP");
    expect(defaultAsSql("mysql", false, "(now() + interval 1 day)", "DEFAULT_GENERATED", "datetime")).toBe("(now() + interval 1 day)");
    expect(defaultAsSql("mysql", true, "'abc'", "", "varchar(10)")).toBe("'abc'");
    expect(defaultAsSql("mysql", true, "NULL", "", "varchar(10)")).toBeNull();
    expect(defaultAsSql("postgres", false, "'x'::character varying", null, "varchar")).toBe("'x'::character varying");
  });
});
