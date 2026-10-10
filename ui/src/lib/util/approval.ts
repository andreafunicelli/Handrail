import type { ApprovalMode, Environment } from "../api/types";

/**
 * Guard for switching approval mode. Auto on a prod connection requires the user to type the
 * connection name exactly (spec §4/§14: prod cannot be Auto silently).
 */
export function needsTypedConfirm(env: Environment, mode: ApprovalMode): boolean {
  return env === "prod" && mode === "auto";
}

export function canSetApprovalMode(
  env: Environment,
  mode: ApprovalMode,
  connectionName: string,
  typed: string | null,
): boolean {
  if (!needsTypedConfirm(env, mode)) return true;
  return typed != null && connectionName.length > 0 && typed.trim() === connectionName;
}
