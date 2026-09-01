/**
 * Generate the "Drivecord folder" icon shown in Windows Explorer for the sync
 * root and each drive subfolder.
 *
 *   scripts/gen-folder-icon.mjs  ->  src-tauri/icons/drivecord-folder.ico
 *
 * A brand-gradient folder shape (indigo -> fuchsia, matching the installer art)
 * with the rounded Drivecord logo composited on top. `sharp` rasterises the
 * SVG; the .ico is assembled here (PNG-in-ICO, valid since Windows Vista) at
 * 16 / 32 / 48 / 256 px.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const ROOT = path.resolve(fileURLToPath(new URL(".", import.meta.url)), "..");
const LOGO = path.join(ROOT, "src-tauri", "icons", "icon.png");
const OUT = path.join(ROOT, "src-tauri", "icons", "drivecord-folder.ico");

const require = createRequire(path.join(ROOT, "..", "discloud", "package.json"));
const sharp = require("sharp");

const SIZES = [16, 32, 48, 256];

async function renderPng(size) {
  const logo = await sharp(LOGO).resize(96, 96, { fit: "contain" }).png().toBuffer();
  const logoB64 = logo.toString("base64");
  // Folder body sits ~x8..x248, y40..x212 on a 256 grid; logo centred on it.
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#6366f1"/>
      <stop offset="1" stop-color="#d946ef"/>
    </linearGradient>
    <linearGradient id="tab" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#818cf8"/>
      <stop offset="1" stop-color="#6366f1"/>
    </linearGradient>
  </defs>
  <path d="M20 46h58l18 20h96a14 14 0 0 1 14 14v6H8v-32a8 8 0 0 1 8-8z" fill="url(#tab)"/>
  <rect x="8" y="72" width="240" height="150" rx="16" fill="url(#g)"/>
  <image x="80" y="94" width="96" height="96" href="data:image/png;base64,${logoB64}"/>
</svg>`;
  return sharp(Buffer.from(svg)).resize(size, size).png().toBuffer();
}

function buildIco(pngs) {
  // pngs: [{ size, buf }]
  const count = pngs.length;
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(count, 4);

  const dir = Buffer.alloc(16 * count);
  let offset = 6 + 16 * count;
  const bodies = [];
  pngs.forEach((p, i) => {
    const d = dir.subarray(i * 16, i * 16 + 16);
    d.writeUInt8(p.size >= 256 ? 0 : p.size, 0); // width  (0 = 256)
    d.writeUInt8(p.size >= 256 ? 0 : p.size, 1); // height
    d.writeUInt8(0, 2); // palette
    d.writeUInt8(0, 3); // reserved
    d.writeUInt16LE(1, 4); // colour planes
    d.writeUInt16LE(32, 6); // bits per pixel
    d.writeUInt32LE(p.buf.length, 8);
    d.writeUInt32LE(offset, 12);
    offset += p.buf.length;
    bodies.push(p.buf);
  });

  return Buffer.concat([header, dir, ...bodies]);
}

async function main() {
  const pngs = [];
  for (const size of SIZES) {
    pngs.push({ size, buf: await renderPng(size) });
  }
  fs.writeFileSync(OUT, buildIco(pngs));
  console.log(`folder icon -> ${path.relative(ROOT, OUT)}  (${fs.statSync(OUT).size} B)`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
