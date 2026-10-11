import type { S3Account } from "../api/types";

/** Starting points for the account editor; the user fills in their endpoint and region. */
export interface S3Preset {
  id: "aws" | "r2" | "b2" | "wasabi" | "other";
  label: string;
  /** Example endpoint (placeholder); empty for AWS. */
  endpoint: string;
  region: string;
  pathStyle: boolean;
}

export const S3_PRESETS: S3Preset[] = [
  { id: "aws", label: "Amazon S3", endpoint: "", region: "us-east-1", pathStyle: false },
  { id: "r2", label: "Cloudflare R2", endpoint: "https://<account-id>.r2.cloudflarestorage.com", region: "auto", pathStyle: true },
  { id: "b2", label: "Backblaze B2", endpoint: "https://s3.us-west-004.backblazeb2.com", region: "us-west-004", pathStyle: true },
  { id: "wasabi", label: "Wasabi", endpoint: "https://s3.eu-central-1.wasabisys.com", region: "eu-central-1", pathStyle: true },
  { id: "other", label: "MinIO / other", endpoint: "http://localhost:9000", region: "us-east-1", pathStyle: true },
];

/** The preset an account looks like it was made from. */
export function presetOf(a: S3Account): S3Preset {
  const e = a.endpoint.toLowerCase();
  const find = (id: S3Preset["id"]) => S3_PRESETS.find((p) => p.id === id)!;
  if (!e.trim()) return find("aws");
  if (e.includes("r2.cloudflarestorage.com")) return find("r2");
  if (e.includes("backblazeb2.com")) return find("b2");
  if (e.includes("wasabisys.com")) return find("wasabi");
  return find("other");
}

/** Short description of where an account points, for lists. */
export function s3Where(a: S3Account): string {
  const host = a.endpoint.trim() ? a.endpoint.replace(/^\w+:\/\//, "").replace(/\/.*$/, "") : `s3.${a.region || "us-east-1"}`;
  return a.bucket?.trim() ? `${a.bucket.trim()} · ${host}` : host;
}
