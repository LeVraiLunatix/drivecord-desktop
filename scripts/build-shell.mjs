/**
 * Builds Drivecord's static "shell" from the sibling `discloud` repo and copies
 * it into `./frontend`, which `tauri.conf.json` uses as `frontendDist`.
 *
 *   node scripts/build-shell.mjs
 *
 * Set DISCLOUD_DIR to point elsewhere (default: ../discloud). Skips the rebuild
 * when SKIP_SHELL_BUILD=1 and ./frontend already exists (fast local iteration).
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(fileURLToPath(import.meta.url), "../..");
const OUT = path.join(ROOT, "frontend");
const DISCLOUD = path.resolve(ROOT, process.env.DISCLOUD_DIR ?? "../discloud");

if (process.env.SKIP_SHELL_BUILD === "1" && fs.existsSync(path.join(OUT, "index.html"))) {
  console.log("build-shell: SKIP_SHELL_BUILD=1 and ./frontend exists — skipping.");
  process.exit(0);
}

if (!fs.existsSync(path.join(DISCLOUD, "scripts", "build-desktop.mjs"))) {
  console.error(`build-shell: discloud not found at ${DISCLOUD} (set DISCLOUD_DIR).`);
  process.exit(1);
}

console.log(`build-shell: building shell in ${DISCLOUD} …`);
execFileSync(process.execPath, [path.join(DISCLOUD, "scripts", "build-desktop.mjs")], {
  cwd: DISCLOUD,
  stdio: "inherit",
});

fs.rmSync(OUT, { recursive: true, force: true });
fs.cpSync(path.join(DISCLOUD, "out"), OUT, { recursive: true });
console.log(`build-shell: copied → ${OUT}`);
