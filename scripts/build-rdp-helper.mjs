// Build the RDP helper (rdp-helper/, its own Cargo workspace) and put it where Tauri's
// `externalBin` expects it: src-tauri/binaries/handrail-rdp-<target triple>[.exe].
//
//   node scripts/build-rdp-helper.mjs                       host target, release build
//   node scripts/build-rdp-helper.mjs --target universal-apple-darwin
//   node scripts/build-rdp-helper.mjs --placeholder         an empty file, for builds that only
//                                                           check or test the app
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};

const hostTriple = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (\S+)$/m)[1];
const target = opt("--target") ?? hostTriple;
const exe = target.includes("windows") ? ".exe" : "";
const outDir = join(root, "src-tauri", "binaries");
const out = join(outDir, `handrail-rdp-${target}${exe}`);
mkdirSync(outDir, { recursive: true });

if (args.includes("--placeholder")) {
  if (!existsSync(out)) writeFileSync(out, "");
  console.log(`placeholder ${out}`);
  process.exit(0);
}

const manifest = join(root, "rdp-helper", "Cargo.toml");
const build = (triple) => {
  execFileSync("cargo", ["build", "--release", "--locked", "--manifest-path", manifest, "--target", triple], { stdio: "inherit" });
  return join(root, "rdp-helper", "target", triple, "release", `handrail-rdp${exe}`);
};

if (target === "universal-apple-darwin") {
  // Tauri builds each architecture on its own (and wants that one's helper), then bundles the
  // universal one.
  const triples = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
  const parts = triples.map(build);
  triples.forEach((t, i) => copyFileSync(parts[i], join(outDir, `handrail-rdp-${t}`)));
  execFileSync("lipo", ["-create", "-output", out, ...parts], { stdio: "inherit" });
} else {
  copyFileSync(build(target), out);
}
console.log(`built ${out}`);
