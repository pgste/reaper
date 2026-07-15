#!/usr/bin/env node
// Assemble a publishable-shaped npm package from wasm-bindgen output.
//
//   node scripts/package-npm.mjs <bindings-dir> [--target nodejs|web]
//
// Writes package.json (version taken from Cargo.toml — single source of
// truth) and copies README.md into the bindings dir, so the directory is
// `npm publish`-shaped. "private": true mirrors the workspace's
// publish = false policy; flipping it is a release decision, not a build
// step.

import { readFileSync, writeFileSync, copyFileSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const crateRoot = join(here, "..");

const outDir = resolve(process.argv[2] ?? join(crateRoot, "pkg-node"));
const target = process.argv.includes("--target")
  ? process.argv[process.argv.indexOf("--target") + 1]
  : "nodejs";

const cargo = readFileSync(join(crateRoot, "Cargo.toml"), "utf8");
const version = /\nversion\s*=\s*"([^"]+)"/.exec(cargo)?.[1];
if (!version) throw new Error("could not read version from Cargo.toml");

for (const f of ["reaper_wasm.js", "reaper_wasm_bg.wasm", "reaper_wasm.d.ts"]) {
  if (!existsSync(join(outDir, f))) {
    throw new Error(`${f} missing in ${outDir} — run wasm-bindgen first`);
  }
}

const pkg = {
  name: target === "web" ? "@reaper/wasm-web" : "@reaper/wasm",
  version,
  description:
    "Reaper policy evaluation core compiled to WebAssembly — sub-microsecond DSL authorization, embeddable without the agent",
  license: "MIT OR Apache-2.0",
  private: true,
  main: "reaper_wasm.js",
  types: "reaper_wasm.d.ts",
  ...(target === "web" ? { type: "module", module: "reaper_wasm.js" } : {}),
  files: ["reaper_wasm.js", "reaper_wasm_bg.wasm", "reaper_wasm.d.ts", "reaper_wasm_bg.wasm.d.ts"],
  keywords: ["authorization", "policy", "rbac", "abac", "rebac", "wasm", "reaper"],
  engines: target === "web" ? undefined : { node: ">=18" },
};

writeFileSync(join(outDir, "package.json"), JSON.stringify(pkg, null, 2) + "\n");
copyFileSync(join(crateRoot, "README.md"), join(outDir, "README.md"));
console.log(`packaged ${pkg.name}@${version} (${target}) in ${outDir}`);
