// Build the RDP helper (rdp-helper/, its own Cargo workspace) and put it where Tauri's
// `externalBin` expects it: src-tauri/binaries/handrail-rdp-<target triple>[.exe].
//
//   node scripts/build-rdp-helper.mjs                       host target, release build
//   node scripts/build-rdp-helper.mjs --target universal-apple-darwin
//   node scripts/build-rdp-helper.mjs --placeholder         an empty file, for builds that only
//                                                           check or test the app
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
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
  const msvc = triple.endsWith("-windows-msvc");
  const env = { ...process.env };
  if (msvc) {
    // The C runtime linked in, as tauri-build does for the app itself: otherwise the helper needs
    // the Visual C++ Redistributable (vcruntime140.dll), which many PCs lack, and RDP fails to
    // start there while the rest of Handrail works. RUSTFLAGS, when set, replaces the per-target
    // flags, so the flag goes to whichever one cargo reads.
    const key = env.RUSTFLAGS != null ? "RUSTFLAGS" : `CARGO_TARGET_${triple.toUpperCase().replaceAll("-", "_")}_RUSTFLAGS`;
    env[key] = `${env[key] ?? ""} -C target-feature=+crt-static`.trim();
  }
  execFileSync("cargo", ["build", "--release", "--locked", "--manifest-path", manifest, "--target", triple], { stdio: "inherit", env });
  const bin = join(root, "rdp-helper", "target", triple, "release", `handrail-rdp${exe}`);
  if (msvc) {
    // Imported DLL names are plain ASCII in the PE file: none of the redistributable's may appear.
    const text = readFileSync(bin).toString("latin1").toLowerCase();
    const dlls = ["vcruntime140", "msvcp140", "api-ms-win-crt-"].filter((dll) => text.includes(dll));
    if (dlls.length) throw new Error(`${bin} still needs the Visual C++ runtime (${dlls.join(", ")})`);
  }
  return bin;
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
