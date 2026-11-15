#!/usr/bin/env node
/**
 * Writes a Tauri v2 updater fragment for one OS family.
 *
 * Usage:
 *   node .github/write-latest-json.mjs <version> <bundle-dir> <platform-key> [out-file]
 *
 * platform-key examples:
 *   windows-x86_64 | linux-x86_64 | linux-aarch64 | darwin-aarch64 | darwin-x86_64
 *
 * Download URLs point at the GitHub Release for RELEASE_TAG (or
 * UPDATER_DOWNLOAD_BASE_URL when set).
 */
import { readFileSync, writeFileSync, readdirSync, existsSync } from "node:fs";
import path from "node:path";

const version = process.argv[2];
const bundleDir = process.argv[3];
const platformKey = process.argv[4];
const outFile = process.argv[5];

if (!version || !bundleDir || !platformKey) {
  console.error(
    "Usage: node .github/write-latest-json.mjs <version> <bundle-dir> <platform-key> [out-file]",
  );
  process.exit(1);
}

function walk(dir) {
  if (!existsSync(dir)) return [];
  const entries = readdirSync(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) files.push(...walk(full));
    else files.push(full);
  }
  return files;
}

const allFiles = walk(bundleDir);
const basenames = allFiles.map((f) => ({ full: f, name: path.basename(f) }));

function findSigned(predicate) {
  const hit = basenames.find(
    (f) => predicate(f.name) && !f.name.endsWith(".sig") && existsSync(`${f.full}.sig`),
  );
  return hit ?? null;
}

function pickArtifacts() {
  const found = [];

  if (platformKey.startsWith("linux-")) {
    const appImage =
      findSigned((n) => /\.AppImage\.tar\.gz$/i.test(n)) ??
      findSigned((n) => /\.AppImage$/i.test(n));
    if (appImage) found.push({ key: platformKey, artifact: appImage });

    const rpm = findSigned((n) => /\.rpm$/i.test(n));
    if (rpm) found.push({ key: `${platformKey}-rpm`, artifact: rpm });

    const deb = findSigned((n) => /\.deb$/i.test(n));
    if (deb) found.push({ key: `${platformKey}-deb`, artifact: deb });

    return found;
  }

  if (platformKey.startsWith("windows-")) {
    const nsis =
      findSigned((n) => /\.nsis\.zip$/i.test(n)) ??
      findSigned((n) => /-setup\.exe$/i.test(n));
    if (nsis) {
      found.push({ key: platformKey, artifact: nsis });
      return found;
    }
    const msi = findSigned((n) => /\.msi$/i.test(n));
    if (msi) found.push({ key: platformKey, artifact: msi });
    return found;
  }

  const app =
    findSigned((n) => /\.app\.tar\.gz$/i.test(n)) ??
    findSigned((n) => /\.tar\.gz$/i.test(n));
  if (app) found.push({ key: platformKey, artifact: app });
  return found;
}

const picked = pickArtifacts();
if (picked.length === 0) {
  console.error("No signed updater artifact found under", bundleDir);
  console.error(
    "Files present:",
    basenames.map((f) => f.name).sort().join(", ") || "(none)",
  );
  process.exit(1);
}

const repo = process.env.GITHUB_REPOSITORY || "rexsystems/tarn";
const tag = process.env.RELEASE_TAG || `v${version.split("+")[0]}`;
const baseUrl =
  process.env.UPDATER_DOWNLOAD_BASE_URL ||
  `https://github.com/${repo}/releases/download/${tag}`;

const platforms = {};
for (const { key, artifact } of picked) {
  const signature = readFileSync(`${artifact.full}.sig`, "utf8").trim();
  const fileUrl = `${baseUrl.replace(/\/+$/, "")}/${artifact.name}`;
  platforms[key] = {
    url: fileUrl,
    signature,
  };
  console.log("  ", key, "←", fileUrl);
}

const fragment = {
  version,
  notes: `Tarn ${version}`,
  pub_date: new Date().toISOString(),
  platforms,
};

const out = outFile ?? path.join(bundleDir, "latest-fragment.json");
writeFileSync(out, `${JSON.stringify(fragment, null, 2)}\n`);
console.log("Wrote", out, "→", Object.keys(platforms).join(", "));
