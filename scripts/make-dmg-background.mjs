#!/usr/bin/env node
/**
 * Bake packaging/macos/dmg-background.png from Tarn brand colors.
 * Pure Node (zlib) - no sharp/canvas. Run anytime the look needs a refresh.
 */
import { deflateSync } from 'node:zlib';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const outDir = join(__dirname, '..', 'packaging', 'macos');
const outPath = join(outDir, 'dmg-background.png');

// Finder DMG window content area (~660×400 logical, @2x pixels).
const W = 1320;
const H = 800;

const bg = [12, 11, 10]; // --bg #0c0b0a
const elev = [24, 23, 21]; // warm lift
const accent = [224, 164, 90]; // --accent #e0a45a
const cream = [245, 243, 240];

function lerp(a, b, t) {
  return a + (b - a) * t;
}
function mix(c0, c1, t) {
  return [
    Math.round(lerp(c0[0], c1[0], t)),
    Math.round(lerp(c0[1], c1[1], t)),
    Math.round(lerp(c0[2], c1[2], t)),
  ];
}
function clamp01(x) {
  return Math.max(0, Math.min(1, x));
}

const row = Buffer.alloc(1 + W * 3);
const raw = Buffer.alloc((H) * (1 + W * 3));

for (let y = 0; y < H; y++) {
  const v = y / (H - 1);
  // Vertical wash: slightly lighter mid, darker edges (purpose: ground the icons).
  const base = mix(bg, elev, Math.sin(Math.PI * v) * 0.35);
  for (let x = 0; x < W; x++) {
    const u = x / (W - 1);
    // Soft amber pool under the drag path, not a full-page glow.
    const cx = 0.5;
    const cy = 0.58;
    const dx = (u - cx) * 1.35;
    const dy = (v - cy) * 1.8;
    const glow = Math.exp(-(dx * dx + dy * dy) * 3.2) * 0.14;

    // Faint arc "drag" cue between icon slots (~180 and ~480 in 660 space).
    const left = 180 / 660;
    const right = 480 / 660;
    const midY = 210 / 400;
    const along = clamp01((u - left) / (right - left));
    const arcY = midY - Math.sin(along * Math.PI) * 0.04;
    const arcDist = Math.hypot((u - (left + along * (right - left))) * W, (v - arcY) * H);
    const arc = Math.exp(-(arcDist * arcDist) / 90) * (along > 0.08 && along < 0.92 ? 0.22 : 0);

    // Tiny chevron tips near the right of the arc.
    let tip = 0;
    if (along > 0.72 && along < 0.9) {
      const local = (along - 0.72) / 0.18;
      const tipY = arcY + (local - 0.5) * 0.03;
      const tipDist = Math.hypot((u - (left + along * (right - left))) * W, (v - tipY) * H);
      tip = Math.exp(-(tipDist * tipDist) / 40) * 0.18;
    }

    let c = mix(base, accent, glow);
    c = mix(c, cream, Math.min(1, arc + tip) * 0.55);

    // Soft vignette so the window edge reads intentional.
    const vig = Math.max(Math.abs(u - 0.5) * 1.15, Math.abs(v - 0.5) * 1.05);
    c = mix(c, bg, clamp01(vig - 0.55) * 0.55);

    const o = 1 + x * 3;
    row[o] = c[0];
    row[o + 1] = c[1];
    row[o + 2] = c[2];
  }
  row[0] = 0; // filter None
  row.copy(raw, y * row.length);
}

function crc32(buf) {
  let c = ~0;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i];
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
  }
  return ~c >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, 'ascii');
  const crcBuf = Buffer.alloc(4);
  const crc = crc32(Buffer.concat([typeBuf, data]));
  crcBuf.writeUInt32BE(crc, 0);
  return Buffer.concat([len, typeBuf, data, crcBuf]);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(W, 0);
ihdr.writeUInt32BE(H, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 2; // RGB
ihdr[10] = 0;
ihdr[11] = 0;
ihdr[12] = 0;

const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk('IHDR', ihdr),
  chunk('IDAT', deflateSync(raw, { level: 9 })),
  chunk('IEND', Buffer.alloc(0)),
]);

mkdirSync(outDir, { recursive: true });
writeFileSync(outPath, png);
console.log(`wrote ${outPath} (${W}x${H}, ${png.length} bytes)`);
