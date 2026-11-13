#!/usr/bin/env node
// Download an official yt-dlp standalone binary into src-tauri/binaries/,
// named for Tauri's externalBin convention: yt-dlp-<target-triple>[.exe]
//
// Official assets: https://github.com/yt-dlp/yt-dlp/releases
//   yt-dlp_linux, yt-dlp_linux_aarch64, yt-dlp.exe, yt-dlp_macos

import { createHash } from "node:crypto";
import { execSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DEST_DIR = join(ROOT, "src-tauri", "binaries");
const RELEASE = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
const UA = "Tarn/0.1.0 (+https://github.com/rexsystems/tarn)";

const ASSETS = {
  "x86_64-unknown-linux-gnu": "yt-dlp_linux",
  "aarch64-unknown-linux-gnu": "yt-dlp_linux_aarch64",
  "armv7-unknown-linux-gnueabihf": "yt-dlp_linux_armv7l",
  "x86_64-unknown-linux-musl": "yt-dlp_linux",
  "aarch64-unknown-linux-musl": "yt-dlp_linux_aarch64",
  "x86_64-pc-windows-msvc": "yt-dlp.exe",
  "x86_64-pc-windows-gnu": "yt-dlp.exe",
  "i686-pc-windows-msvc": "yt-dlp.exe",
  "aarch64-pc-windows-msvc": "yt-dlp.exe",
  "x86_64-apple-darwin": "yt-dlp_macos",
  "aarch64-apple-darwin": "yt-dlp_macos",
};

function triple() {
  const env =
    process.env.TAURI_ENV_TARGET_TRIPLE ||
    process.env.CARGO_BUILD_TARGET ||
    process.env.TARGET ||
    "";
  if (env) return env;
  const arg = process.argv.slice(2).find((a) => a.includes("-") && !a.startsWith("-"));
  if (arg) return arg;
  const out = execSync("rustc -vV", { encoding: "utf8" });
  const m = out.match(/host: (\S+)/);
  if (!m) throw new Error("cannot detect target triple (rustc -vV)");
  return m[1];
}

function isMobile(t) {
  return /android|ios|wasm/.test(t);
}

async function fetchBuf(url) {
  const res = await fetch(url, { headers: { "User-Agent": UA }, redirect: "follow" });
  if (!res.ok) throw new Error(`${res.status} ${res.statusText} for ${url}`);
  return Buffer.from(await res.arrayBuffer());
}

async function checksums() {
  try {
    const txt = (await fetchBuf(`${RELEASE}/SHA2-256SUMS`)).toString("utf8");
    const map = new Map();
    for (const line of txt.split("\n")) {
      const m = line.trim().match(/^([0-9a-fA-F]{64})\s+(\S+)$/);
      if (m) map.set(m[2], m[1].toLowerCase());
    }
    return map;
  } catch (e) {
    console.warn("fetch-ytdlp: could not load SHA2-256SUMS:", e.message);
    return new Map();
  }
}

async function main() {
  const t = triple();
  if (isMobile(t)) {
    console.log(`fetch-ytdlp: skipping sidecar for mobile target ${t}`);
    return;
  }
  const asset = ASSETS[t];
  if (!asset) throw new Error(`no official yt-dlp asset for target ${t}`);
  const windows = t.includes("windows");
  const destName = windows ? `yt-dlp-${t}.exe` : `yt-dlp-${t}`;
  mkdirSync(DEST_DIR, { recursive: true });
  const dest = join(DEST_DIR, destName);
  const force = process.argv.includes("--force") || process.env.CI === "true";
  if (!force && existsSync(dest)) {
    const n = readFileSync(dest).length;
    if (n > 1_000_000) {
      console.log(`fetch-ytdlp: ${destName} already present (${n} bytes), skip`);
      return;
    }
  }
  console.log(`fetch-ytdlp: downloading ${asset} for ${t}`);
  const sums = await checksums();
  const buf = await fetchBuf(`${RELEASE}/${asset}`);
  if (buf.length < 1_000_000) {
    throw new Error(`download too small (${buf.length} bytes) — unexpected asset`);
  }
  const want = sums.get(asset);
  if (want) {
    const got = createHash("sha256").update(buf).digest("hex");
    if (got !== want) {
      throw new Error(`SHA-256 mismatch for ${asset}: got ${got}, expected ${want}`);
    }
  }
  writeFileSync(dest, buf);
  if (!windows) chmodSync(dest, 0o755);
  console.log(`fetch-ytdlp: wrote ${dest} (${buf.length} bytes)`);
}

main().catch((e) => {
  console.error("fetch-ytdlp:", e.message || e);
  process.exit(1);
});
