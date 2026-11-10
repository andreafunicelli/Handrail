// Pure parts of the connection editor: which collapsible sections open by themselves, what a
// connection test is sent, and when a test result no longer describes the form.
import type { Connection, ConnTestSecrets, SshAuth } from "../../api/types";
import { tunnelProblem } from "../../util/tunnels";
import { uuid } from "../../util/id";

export type AuthChoice = "password" | "key" | "agent" | "prompt";

const isDb = (c: Pick<Connection, "kind">) => c.kind === "mysql" || c.kind === "postgres";

/** Network holds something other than the defaults (jump hosts, forwards, RDP or TLS options). */
export function networkHasValues(c: Pick<Connection, "kind" | "jump" | "tunnels" | "rdp_admin" | "db_tls">): boolean {
  if (c.jump.length) return true;
  if (c.kind === "ssh") return c.tunnels.length > 0;
  if (c.kind === "rdp") return !!c.rdp_admin;
  if (isDb(c)) return c.db_tls != null;
  return false;
}

/** A field in Network keeps the connection from being saved. */
export function networkHasError(c: Pick<Connection, "kind" | "tunnels">): boolean {
  return c.kind === "ssh" && c.tunnels.some((x) => tunnelProblem(x) !== null);
}

/** Advanced holds something: tags, notes, an approval mode or an icon. */
export function advancedHasValues(c: Pick<Connection, "kind" | "notes" | "approval_mode" | "icon">, tagsText: string): boolean {
  return tagsText.trim() !== "" || c.notes.trim() !== "" || (c.kind !== "rdp" && c.approval_mode != null) || !!c.icon;
}

/** Sections that start open: the ones that hold non-default values. */
export function initialSections(c: Connection, tagsText: string): { network: boolean; advanced: boolean } {
  return { network: networkHasValues(c) || networkHasError(c), advanced: advancedHasValues(c, tagsText) };
}

/** The form's login, as the editor holds it. */
export interface LoginForm {
  authType: AuthChoice;
  identityId: string;
  /** Typed password, and the saved one picked (""). */
  password: string;
  passwordId: string;
  /** Pasted key, the saved one picked, and the saved passphrase that goes with it. */
  keyText: string;
  keyId: string;
  keyPassphraseId: string | null;
  passphrase: string;
  /** Typed for the test of a connection that asks for its password (never saved). */
  testPassword: string;
}

/** What `testConnection` is sent for the form's login, or null when there is nothing to log in
 *  with yet (a key login with no key). Typed secrets go along for the test only; a typed secret's
 *  `SshAuth` points at a fresh id that is not looked up. */
export function testLogin(f: LoginForm): { auth: SshAuth; identity: string | null; secrets: ConnTestSecrets } | null {
  if (f.identityId) return { auth: { type: "prompt" }, identity: f.identityId, secrets: {} };
  const own = (auth: SshAuth, secrets: ConnTestSecrets = {}) => ({ auth, identity: null, secrets });
  switch (f.authType) {
    case "password":
      if (f.password) return own({ type: "password", secret: f.passwordId || uuid() }, { password: f.password });
      if (f.passwordId) return own({ type: "password", secret: f.passwordId });
      return own({ type: "prompt" }, f.testPassword ? { password: f.testPassword } : {});
    case "key":
      if (f.keyText.trim()) return own({ type: "key", key: f.keyId || uuid(), passphrase: null }, { key: f.keyText, ...(f.passphrase ? { passphrase: f.passphrase } : {}) });
      if (f.keyId) return own({ type: "key", key: f.keyId, passphrase: f.keyPassphraseId }, f.passphrase ? { passphrase: f.passphrase } : {});
      return null;
    case "agent":
      return own({ type: "agent" });
    case "prompt":
      return own({ type: "prompt" }, f.testPassword ? { password: f.testPassword } : {});
  }
}

/** A short digest, so the signature holds no typed secret. */
function digest(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193);
  return `${s.length}:${(h >>> 0).toString(36)}`;
}

/** Everything a test result depends on: when it changes, the result shown is stale. */
export function testSignature(c: Pick<Connection, "kind" | "host" | "port" | "user" | "jump" | "database" | "db_tls">, f: LoginForm): string {
  return JSON.stringify([
    c.kind,
    c.host.trim(),
    c.port,
    c.user.trim(),
    c.jump,
    c.database ?? null,
    c.db_tls ?? null,
    f.identityId,
    f.authType,
    digest(f.password),
    f.passwordId,
    digest(f.keyText),
    f.keyId,
    digest(f.passphrase),
    digest(f.testPassword),
  ]);
}
