import { describe, expect, it } from "vitest";
import { splitStatements, statementAt } from "./sqltext";

describe("sql text", () => {
  it("splits on semicolons outside strings, names and comments", () => {
    const sql = "SELECT 'a;b', `c;d`; -- x;y\nSELECT 2 /* ; */;\n\n  ;  SELECT 3";
    expect(splitStatements(sql, "mysql").map((s) => s.text)).toEqual([
      "SELECT 'a;b', `c;d`",
      "-- x;y\nSELECT 2 /* ; */",
      "SELECT 3",
    ]);
    expect(splitStatements("SELECT $$a;b$$; SELECT $t$;$t$, $1", "postgres").map((s) => s.text)).toEqual([
      "SELECT $$a;b$$",
      "SELECT $t$;$t$, $1",
    ]);
    // MySQL: backslash escapes the quote; -- needs a blank after it.
    expect(splitStatements("SELECT 'it\\'s;'; SELECT 1--1;", "mysql").map((s) => s.text)).toEqual(["SELECT 'it\\'s;'", "SELECT 1--1"]);
    expect(splitStatements("-- only a comment\n", "postgres")).toEqual([]);
  });

  it("finds the statement under the cursor", () => {
    const sql = "SELECT 1;\nSELECT 2;\n\nSELECT 3";
    expect(statementAt(sql, 3, "mysql")?.text).toBe("SELECT 1");
    expect(statementAt(sql, sql.indexOf("2"), "mysql")?.text).toBe("SELECT 2");
    // Right after a statement's semicolon, or on the blank line after it: that statement.
    expect(statementAt(sql, sql.indexOf("2;") + 2, "mysql")?.text).toBe("SELECT 2");
    expect(statementAt(sql, sql.indexOf("\n\n") + 1, "mysql")?.text).toBe("SELECT 2");
    expect(statementAt(sql, sql.length, "mysql")?.text).toBe("SELECT 3");
    expect(statementAt("  ", 1, "mysql")).toBeNull();
  });
});
