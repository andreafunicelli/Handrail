import { describe, expect, it } from "vitest";
import {
  DEFAULT_OPSLOG_SETTINGS,
  decisionOf,
  filterEntries,
  operationText,
  outcomeOf,
  parseEntries,
  parseSettings,
  prune,
  redact,
  redactOperation,
  type OpsEntry,
} from "./opslog";

describe("redact", () => {
  const cases: [string, string][] = [
    ["mysql -uroot -pS3cret! mydb", "mysql -uroot -p*** mydb"],
    ["mysql -u root -p mydb", "mysql -u root -p mydb"],
    ["psql postgres://admin:hunter2@db.local:5432/app", "psql postgres://admin:***@db.local:5432/app"],
    ["git clone https://user:ghp_abcdefghijklmnopqrstuvwxyz0123@github.com/x/y", "git clone https://user:***@github.com/x/y"],
    ["mongodump --password=topsecret --db x", "mongodump --password=*** --db x"],
    ["tool --password 'two words' --verbose", "tool --password *** --verbose"],
    ["PGPASSWORD=abc123 psql -h db", "PGPASSWORD=*** psql -h db"],
    ["export MYSQL_PASSWORD=\"p@ss w0rd\"", "export MYSQL_PASSWORD=***"],
    ["curl 'https://api.example.com/v1?token=abcd1234&x=1'", "curl 'https://api.example.com/v1?token=***&x=1'"],
    ["curl -H 'Authorization: Bearer eyJhbGciOi.payload.sig' https://x", "curl -H 'Authorization: Bearer ***' https://x"],
    ["curl -u admin:pa55 https://x", "curl -u admin:*** https://x"],
    ["aws configure set aws_access_key_id AKIAIOSFODNN7EXAMPLE", "aws configure set aws_access_key_id AKIA***"],
    ["AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY aws s3 ls", "AWS_SECRET_ACCESS_KEY=*** aws s3 ls"],
    ["sshpass -p hunter2 ssh host", "sshpass -p *** ssh host"],
    ["CREATE USER bob IDENTIFIED BY 'pw1';", "CREATE USER bob IDENTIFIED BY '***';"],
    ["ALTER ROLE app WITH PASSWORD 'pw2'", "ALTER ROLE app WITH PASSWORD '***'"],
    ["export ANTHROPIC_KEY=sk-ant-api03-abcdefghijklmnopqrstuv", "export ANTHROPIC_KEY=sk-ant-***"],
  ];
  for (const [input, want] of cases) it(input, () => expect(redact(input)).toBe(want));

  it("masks private key blocks", () => {
    const key = "cat > id <<EOF\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nAAAA\n-----END OPENSSH PRIVATE KEY-----\nEOF";
    const r = redact(key);
    expect(r).not.toContain("b3BlbnNzaC1rZXktdjEAAAAA");
    expect(r).toContain("-----BEGIN OPENSSH PRIVATE KEY----- *** -----END OPENSSH PRIVATE KEY-----");
    expect(redact("-----BEGIN RSA PRIVATE KEY-----\nMIIEow")).not.toContain("MIIEow");
  });

  it("leaves ordinary commands alone", () => {
    for (const c of ["df -h", "ls -la /var/log", "systemctl status nginx", "ssh -p 2222 host", "grep -r password_reset src/"])
      expect(redact(c)).toBe(c);
  });

  it("truncates to 300 characters after redacting", () => {
    const long = "echo " + "x".repeat(400) + " PASSWORD=zzz";
    const r = redactOperation(long);
    expect(r.length).toBe(300);
    expect(r.endsWith("…")).toBe(true);
    const tail = redactOperation("x".repeat(280) + " --token=abcdefghijklmnopqrstuvwxyz");
    expect(tail).not.toContain("abcdefghij");
  });
});

const entry = (at: number, extra: Partial<OpsEntry> = {}): OpsEntry => ({
  at,
  session_id: "s",
  connection_id: "c1",
  connection_name: "web-01",
  env: "prod",
  agent: "Claude",
  tool: "run_command",
  operation: "df -h",
  decision: "approved",
  risk: "read",
  outcome: "completed",
  exit_code: 0,
  duration_ms: 10,
  error: null,
  ...extra,
});

describe("retention and cap", () => {
  const now = Date.UTC(2026, 10, 10);
  const day = 86_400_000;

  it("drops entries older than the retention window", () => {
    const list = [entry(now - 40 * day), entry(now - 29 * day), entry(now - 1000)];
    expect(prune(list, { retention_days: 30, max_entries: 1000 }, now).map((e) => e.at)).toEqual([now - 29 * day, now - 1000]);
  });

  it("keeps only the newest max_entries, in time order", () => {
    const list = Array.from({ length: 1200 }, (_, i) => entry(now - (1200 - i) * 1000));
    const kept = prune(list, { retention_days: 30, max_entries: 1000 }, now);
    expect(kept).toHaveLength(1000);
    expect(kept[0].at).toBe(now - 1000 * 1000);
    expect(kept.at(-1)!.at).toBe(now - 1000);
  });

  it("sorts merged batches", () => {
    const kept = prune([entry(now - 5), entry(now - 9), entry(now - 1)], DEFAULT_OPSLOG_SETTINGS, now);
    expect(kept.map((e) => now - e.at)).toEqual([9, 5, 1]);
  });

  it("parses settings defensively", () => {
    expect(parseSettings(null)).toEqual(DEFAULT_OPSLOG_SETTINGS);
    expect(parseSettings({ retention_days: -4, max_entries: 1e9, record_commands: false })).toEqual({
      enabled: true,
      record_commands: false,
      retention_days: 1,
      max_entries: 10000,
    });
    expect(parseEntries([entry(1), { nope: true }, null, "x"])).toHaveLength(1);
    expect(parseEntries({})).toEqual([]);
  });
});

describe("record fields", () => {
  it("decision", () => {
    expect(decisionOf("run_command", { decision: { type: "approve" } }, null)).toBe("approved");
    expect(decisionOf("run_command", { decision: { type: "approve_edited", command: "x" } }, null)).toBe("edited");
    expect(decisionOf("run_command", { decision: { type: "deny", message: null } }, null)).toBe("denied");
    expect(decisionOf("run_command", null, { state: "completed" })).toBe("auto");
    expect(decisionOf("run_command", null, { state: "denied" })).toBe("blocked");
    expect(decisionOf("read_output", null, null)).toBe("none");
  });

  it("outcome and error category", () => {
    expect(outcomeOf({ state: "completed", exit_code: 2, duration_ms: 5 }, false, false)).toEqual({ outcome: "completed", exit_code: 2, duration_ms: 5, error: "exit_code" });
    expect(outcomeOf({ state: "completed", exit_code: 0, duration_ms: 5 }, false, false).error).toBeNull();
    expect(outcomeOf({ state: "timed_out", exit_code: null, duration_ms: 5 }, false, false).error).toBe("timed_out");
    expect(outcomeOf({ outcome: "rejected", path: "/x" }, false, false)).toMatchObject({ outcome: "rejected", error: "rejected" });
    expect(outcomeOf("boom", true, false)).toMatchObject({ outcome: "error", error: "tool_error" });
    expect(outcomeOf(null, false, true)).toMatchObject({ outcome: "no_result", error: "no_result" });
  });

  it("operation text never includes file contents", () => {
    expect(operationText("propose_file_edit", { path: "/etc/x", content: "SECRET" }, null)).toBe("/etc/x");
    expect(operationText("run_command", { command: "ls" }, { command: "ls -la" })).toBe("ls -la");
    expect(operationText("read_output", { session_id: "s", output_id: "o1", offset: 10 }, null)).toBe("output_id=o1 offset=10");
  });

  it("filters", () => {
    const list = [entry(1), entry(2, { connection_id: null, connection_name: null, error: "exit_code", exit_code: 1, decision: "auto" }), entry(3, { operation: "uptime" })];
    expect(filterEntries(list, { connection: "local", decision: "", outcome: "", text: "" })).toHaveLength(1);
    expect(filterEntries(list, { connection: "", decision: "", outcome: "failed", text: "" }).map((e) => e.at)).toEqual([2]);
    expect(filterEntries(list, { connection: "", decision: "approved", outcome: "ok", text: "UPT" }).map((e) => e.at)).toEqual([3]);
  });
});
