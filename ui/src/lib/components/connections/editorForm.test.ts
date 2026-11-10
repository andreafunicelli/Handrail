import { describe, expect, it } from "vitest";
import type { Connection } from "../../api/types";
import { advancedHasValues, initialSections, networkHasError, networkHasValues, testLogin, testSignature, type LoginForm } from "./editorForm";

const base: Connection = {
  id: "c1",
  name: "web",
  kind: "ssh",
  group_id: null,
  host: "10.0.0.1",
  port: 22,
  user: "deploy",
  auth: { type: "agent" },
  jump: [],
  env: "none",
  tags: [],
  color: null,
  approval_mode: null,
  local_shell: null,
  notes: "",
  tunnels: [],
};
const fwd = { kind: "local" as const, bind_host: "127.0.0.1", bind_port: 8080, dest_host: "db", dest_port: 5432, enabled: true };
const form: LoginForm = { authType: "agent", identityId: "", password: "", passwordId: "", keyText: "", keyId: "", keyPassphraseId: null, passphrase: "", testPassword: "" };

describe("editor sections", () => {
  it("start closed for a plain connection", () => {
    expect(initialSections(base, "")).toEqual({ network: false, advanced: false });
  });

  it("open Network for jump hosts, forwards, RDP admin and database TLS", () => {
    expect(networkHasValues({ ...base, jump: ["j"] })).toBe(true);
    expect(networkHasValues({ ...base, tunnels: [fwd] })).toBe(true);
    expect(networkHasValues({ ...base, kind: "rdp", rdp_admin: true })).toBe(true);
    expect(networkHasValues({ ...base, kind: "rdp", rdp_admin: false })).toBe(false);
    expect(networkHasValues({ ...base, kind: "mysql", db_tls: "require" })).toBe(true);
    expect(networkHasValues({ ...base, kind: "postgres", db_tls: null })).toBe(false);
    // Forwards left over from an SSH connection turned into a database are not saved.
    expect(networkHasValues({ ...base, kind: "mysql", tunnels: [fwd] })).toBe(false);
  });

  it("open Network for a forward that cannot be saved", () => {
    const bad = { ...fwd, dest_host: "" };
    expect(networkHasError({ ...base, tunnels: [bad] })).toBe(true);
    expect(networkHasError({ ...base, tunnels: [fwd] })).toBe(false);
    expect(initialSections({ ...base, tunnels: [bad] }, "").network).toBe(true);
  });

  it("open Advanced for tags, notes, an approval mode or an icon", () => {
    expect(advancedHasValues(base, " web ")).toBe(true);
    expect(advancedHasValues({ ...base, notes: "rack 4" }, "")).toBe(true);
    expect(advancedHasValues({ ...base, approval_mode: "auto" }, "")).toBe(true);
    expect(advancedHasValues({ ...base, icon: "database" }, "")).toBe(true);
    expect(advancedHasValues({ ...base, notes: "  " }, "")).toBe(false);
    // RDP has no agent approvals: a leftover mode does not count.
    expect(advancedHasValues({ ...base, kind: "rdp", approval_mode: "auto" }, "")).toBe(false);
  });
});

describe("connection test login", () => {
  it("uses the identity, and nothing typed, when there is one", () => {
    expect(testLogin({ ...form, identityId: "i1", password: "x" })).toEqual({ auth: { type: "prompt" }, identity: "i1", secrets: {} });
  });

  it("sends a typed password for the test only", () => {
    const r = testLogin({ ...form, authType: "password", password: "pw", passwordId: "s1" });
    expect(r?.auth).toEqual({ type: "password", secret: "s1" });
    expect(r?.secrets).toEqual({ password: "pw" });
    const fresh = testLogin({ ...form, authType: "password", password: "pw" });
    expect(fresh?.auth.type).toBe("password");
    expect(testLogin({ ...form, authType: "password", passwordId: "s1" })?.secrets).toEqual({});
  });

  it("asks at connect time without a password, with the test password if typed", () => {
    expect(testLogin({ ...form, authType: "password" })).toEqual({ auth: { type: "prompt" }, identity: null, secrets: {} });
    expect(testLogin({ ...form, authType: "prompt", testPassword: "t" })?.secrets).toEqual({ password: "t" });
  });

  it("sends a pasted key, or the saved one with its passphrase", () => {
    const pasted = testLogin({ ...form, authType: "key", keyText: "PEM", passphrase: "pp" });
    expect(pasted?.secrets).toEqual({ key: "PEM", passphrase: "pp" });
    const saved = testLogin({ ...form, authType: "key", keyId: "k1", keyPassphraseId: "p1" });
    expect(saved?.auth).toEqual({ type: "key", key: "k1", passphrase: "p1" });
    expect(saved?.secrets).toEqual({});
    expect(testLogin({ ...form, authType: "key" })).toBeNull();
  });

  it("goes stale when what it tested changes, and holds no typed secret", () => {
    const sig = testSignature(base, { ...form, password: "hunter2" });
    expect(sig).not.toContain("hunter2");
    expect(testSignature(base, { ...form, password: "hunter2" })).toBe(sig);
    expect(testSignature({ ...base, host: "10.0.0.2" }, { ...form, password: "hunter2" })).not.toBe(sig);
    expect(testSignature(base, { ...form, password: "hunter3" })).not.toBe(sig);
    // Name, tags and notes do not change what a test does.
    const renamed: Connection = { ...base, name: "other", notes: "n", tags: ["x"] };
    expect(testSignature(renamed, { ...form, password: "hunter2" })).toBe(sig);
  });
});
