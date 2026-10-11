/** `ls -l` style mode string, e.g. 0o40755 → "drwxr-xr-x". */
export function modeString(mode: number | null): string {
  if (mode == null) return "";
  const type = { 0o040000: "d", 0o120000: "l", 0o100000: "-" }[mode & 0o170000] ?? "?";
  const bits = "rwxrwxrwx";
  let out = type;
  for (let i = 0; i < 9; i++) out += mode & (1 << (8 - i)) ? bits[i] : "-";
  return out;
}

/** Parent of a POSIX path ("/" stays "/"). */
export function parentDir(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const i = trimmed.lastIndexOf("/");
  return i <= 0 ? "/" : trimmed.slice(0, i);
}

/** `dir/name` with exactly one slash. */
export function joinPath(dir: string, name: string): string {
  return dir.endsWith("/") ? dir + name : `${dir}/${name}`;
}

/** A single path segment: no slashes, not "." or "..". */
export function validName(name: string): boolean {
  const n = name.trim();
  return !!n && n !== "." && n !== ".." && !n.includes("/");
}
