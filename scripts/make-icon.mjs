/**
 * Generates src-tauri/icons/source.png — the master icon that `npx tauri icon`
 * expands into every platform size.
 *
 * Upstream's menu bar icon is a plain circle; the recording state turns it red.
 * We keep that motif: a dark rounded square with a ring, so the tray icon stays
 * recognisable at 16 px.
 *
 * No dependencies: writes the PNG (IHDR/IDAT/IEND) directly via node:zlib.
 */
import { deflateSync, crc32 } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

const SIZE = 1024;
const SS = 2; // supersampling factor per axis

// --- geometry -------------------------------------------------------------

const bg = { r: 0x18, g: 0x18, b: 0x1a };
const ring = { r: 0xf2, g: 0xf2, b: 0xf5 };
const dot = { r: 0xff, g: 0x45, b: 0x3a };

const radius = SIZE * 0.22; // rounded-square corner
const cx = SIZE / 2;
const cy = SIZE / 2;
const ringOuter = SIZE * 0.30;
const ringInner = SIZE * 0.225;
const dotRadius = SIZE * 0.085;

function insideRoundedSquare(x, y) {
  const h = SIZE / 2;
  const qx = Math.abs(x - h) - (h - radius);
  const qy = Math.abs(y - h) - (h - radius);
  const outside = Math.hypot(Math.max(qx, 0), Math.max(qy, 0));
  const inside = Math.min(Math.max(qx, qy), 0);
  return outside + inside - radius <= 0;
}

/** Returns {r,g,b,a} for one sample point (a in 0..255). */
function sample(x, y) {
  if (!insideRoundedSquare(x, y)) return { ...bg, a: 0 };
  const d = Math.hypot(x - cx, y - cy);
  if (d <= dotRadius) return { ...dot, a: 255 };
  if (d >= ringInner && d <= ringOuter) return { ...ring, a: 255 };
  return { ...bg, a: 255 };
}

// --- raster -----------------------------------------------------------------

const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));
let p = 0;
for (let y = 0; y < SIZE; y++) {
  raw[p++] = 0; // filter: none
  for (let x = 0; x < SIZE; x++) {
    let r = 0;
    let g = 0;
    let b = 0;
    let a = 0;
    for (let sy = 0; sy < SS; sy++) {
      for (let sx = 0; sx < SS; sx++) {
        const c = sample(x + (sx + 0.5) / SS, y + (sy + 0.5) / SS);
        // Premultiply so edge pixels do not fringe against the dark background.
        const w = c.a / 255;
        r += c.r * w;
        g += c.g * w;
        b += c.b * w;
        a += c.a;
      }
    }
    const n = SS * SS;
    const av = a / n;
    const w = av / 255;
    raw[p++] = w > 0 ? Math.round(r / n / w) : 0;
    raw[p++] = w > 0 ? Math.round(g / n / w) : 0;
    raw[p++] = w > 0 ? Math.round(b / n / w) : 0;
    raw[p++] = Math.round(av);
  }
}

// --- PNG container ----------------------------------------------------------

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body) >>> 0);
  return Buffer.concat([len, body, crc]);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // colour type: RGBA
ihdr[10] = 0; // deflate
ihdr[11] = 0; // adaptive filtering
ihdr[12] = 0; // no interlace

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

const out = "src-tauri/icons/source.png";
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, png);
console.log(`wrote ${out} (${SIZE}x${SIZE}, ${png.length} bytes)`);
