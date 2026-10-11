import { isIpcError } from "../api/errors";

/** `aht_import::Tool` slugs, in the order the file picker lists them. */
export const IMPORT_TOOLS = ["ssh_config", "putty", "mobaxterm", "mremoteng", "winscp", "securecrt", "tabby"] as const;
export type ImportTool = (typeof IMPORT_TOOLS)[number];

const LABELS: Record<ImportTool, string> = {
  ssh_config: "OpenSSH",
  putty: "PuTTY",
  mobaxterm: "MobaXterm",
  mremoteng: "mRemoteNG",
  winscp: "WinSCP",
  securecrt: "SecureCRT",
  tabby: "Tabby",
};

/** Display name for a tool slug; unknown values (e.g. the mock's labels) pass through. */
export function toolLabel(tool: string): string {
  return (LABELS as Record<string, string>)[tool] ?? tool;
}


/**
 * mRemoteNG files protected with a custom master password fail with an `auth` error; the UI
 * then asks for that password and retries.
 */
export function needsMremotengPassword(tool: string, e: unknown): boolean {
  return tool === "mremoteng" && isIpcError(e) && e.kind === "auth";
}
