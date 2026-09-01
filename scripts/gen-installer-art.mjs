/**
 * Generate the branded NSIS installer artwork from the rounded Drivecord logo.
 *
 *   scripts/gen-installer-art.mjs
 *
 * Writes to src-tauri/installer/ :
 *   - header.bmp   150 x 57   (MUI_HEADERIMAGE_BITMAP  — top banner)
 *   - sidebar.bmp  164 x 314  (MUI_WELCOMEFINISHPAGE_BITMAP — welcome / finish)
 *   - license.txt  (short FR notice shown on the license page)
 *
 * icon.ico is reused straight from src-tauri/icons/icon.ico (already the
 * rounded logo), so this script does not touch it.
 *
 * NSIS wants uncompressed 24-bit BMP. `sharp` can't encode BMP, so we build the
 * pixels ourselves: a vertical brand gradient (indigo -> fuchsia, matching the
 * app's `desktop-sync` screen) with the logo alpha-composited on top, then a
 * hand-rolled BITMAPINFOHEADER. No SVG / no system fonts involved.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const ROOT = path.resolve(fileURLToPath(new URL(".", import.meta.url)), "..");
const OUT = path.join(ROOT, "src-tauri", "installer");
const LOGO = path.join(ROOT, "src-tauri", "icons", "icon.png");

// `sharp` lives in the sibling discloud install (this repo has no bundler dep).
const require = createRequire(path.join(ROOT, "..", "discloud", "package.json"));
const sharp = require("sharp");

// Brand gradient stops (Tailwind indigo-500 -> fuchsia-500).
const TOP = [0x63, 0x66, 0xf1];
const BOTTOM = [0xd9, 0x46, 0xef];

const lerp = (a, b, t) => Math.round(a + (b - a) * t);

/** Build a W*H RGB buffer: vertical gradient, then composite `logo` (RGBA). */
function compose(w, h, logo, lw, lh, lx, ly) {
  const rgb = Buffer.alloc(w * h * 3);
  for (let y = 0; y < h; y++) {
    const t = h === 1 ? 0 : y / (h - 1);
    const r = lerp(TOP[0], BOTTOM[0], t);
    const g = lerp(TOP[1], BOTTOM[1], t);
    const b = lerp(TOP[2], BOTTOM[2], t);
    for (let x = 0; x < w; x++) {
      const o = (y * w + x) * 3;
      rgb[o] = r;
      rgb[o + 1] = g;
      rgb[o + 2] = b;
    }
  }
  // Alpha-blend the logo.
  for (let y = 0; y < lh; y++) {
    const py = ly + y;
    if (py < 0 || py >= h) continue;
    for (let x = 0; x < lw; x++) {
      const px = lx + x;
      if (px < 0 || px >= w) continue;
      const s = (y * lw + x) * 4;
      const a = logo[s + 3] / 255;
      if (a === 0) continue;
      const o = (py * w + px) * 3;
      rgb[o] = Math.round(logo[s] * a + rgb[o] * (1 - a));
      rgb[o + 1] = Math.round(logo[s + 1] * a + rgb[o + 1] * (1 - a));
      rgb[o + 2] = Math.round(logo[s + 2] * a + rgb[o + 2] * (1 - a));
    }
  }
  return rgb;
}

/** Encode a top-left-origin RGB buffer as an uncompressed 24-bit BMP. */
function encodeBmp(rgb, w, h) {
  const rowSize = Math.ceil((w * 3) / 4) * 4;
  const pixels = Buffer.alloc(rowSize * h);
  for (let y = 0; y < h; y++) {
    const srcRow = y * w * 3;
    const dstRow = (h - 1 - y) * rowSize; // BMP is bottom-up
    for (let x = 0; x < w; x++) {
      const s = srcRow + x * 3;
      const d = dstRow + x * 3;
      pixels[d] = rgb[s + 2]; // B
      pixels[d + 1] = rgb[s + 1]; // G
      pixels[d + 2] = rgb[s]; // R
    }
  }
  const header = Buffer.alloc(54);
  header.write("BM", 0, "ascii");
  header.writeUInt32LE(54 + pixels.length, 2);
  header.writeUInt32LE(54, 10); // pixel data offset
  header.writeUInt32LE(40, 14); // DIB header size
  header.writeInt32LE(w, 18);
  header.writeInt32LE(h, 22);
  header.writeUInt16LE(1, 26); // planes
  header.writeUInt16LE(24, 28); // bpp
  header.writeUInt32LE(0, 30); // BI_RGB
  header.writeUInt32LE(pixels.length, 34);
  header.writeInt32LE(2835, 38); // 72 DPI
  header.writeInt32LE(2835, 42);
  return Buffer.concat([header, pixels]);
}

async function logoRaw(size) {
  const { data } = await sharp(LOGO)
    .resize(size, size, { fit: "contain", background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  return data; // RGBA, size*size*4
}

async function main() {
  fs.mkdirSync(OUT, { recursive: true });

  // Header 150x57 — logo tucked to the right (MUI header text sits on the left).
  {
    const w = 150;
    const h = 57;
    const s = 43;
    const logo = await logoRaw(s);
    const rgb = compose(w, h, logo, s, s, w - s - 8, Math.round((h - s) / 2));
    fs.writeFileSync(path.join(OUT, "header.bmp"), encodeBmp(rgb, w, h));
  }

  // Sidebar 164x314 — logo centred in the upper third.
  {
    const w = 164;
    const h = 314;
    const s = 116;
    const logo = await logoRaw(s);
    const rgb = compose(w, h, logo, s, s, Math.round((w - s) / 2), 64);
    fs.writeFileSync(path.join(OUT, "sidebar.bmp"), encodeBmp(rgb, w, h));
  }

  fs.writeFileSync(
    path.join(OUT, "license.txt"),
    [
      "Drivecord Desktop",
      "",
      "Client Windows non officiel pour Drivecord (drivecord.app).",
      "Distribue sous licence MIT.",
      "",
      "Code source : https://github.com/LeVraiLunatix/drivecord-desktop",
      "",
      "En installant ce logiciel, vous acceptez la licence MIT ci-dessus.",
      "",
    ].join("\r\n"),
  );

  console.log("installer art -> " + path.relative(ROOT, OUT));
  for (const f of ["header.bmp", "sidebar.bmp", "license.txt"]) {
    const st = fs.statSync(path.join(OUT, f));
    console.log(`  ${f}  ${st.size} B`);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
