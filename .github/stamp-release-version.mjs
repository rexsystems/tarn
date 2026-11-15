#!/usr/bin/env node
/**
 * Stamp release version as "<base>+<build>" for CI.
 *
 * Updates tauri.conf.json, Cargo.toml, and package.json so the installed
 * app version is e.g. 0.1.4+1.
 *
 * Usage:
 *   node .github/stamp-release-version.mjs <build-number>
 */
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const build = process.argv[2];
if (!build || !/^\d+$/.test(build)) {
  console.error("Usage: node .github/stamp-release-version.mjs <build-number>");
  process.exit(1);
}

function readJson(filePath) {
  return JSON.parse(readFileSync(filePath, "utf8"));
}

function writeJson(filePath, data) {
  writeFileSync(filePath, `${JSON.stringify(data, null, 2)}\n`);
}

const confPath = path.resolve("src-tauri/tauri.conf.json");
const cargoPath = path.resolve("src-tauri/Cargo.toml");
const rootPkgPath = path.resolve("package.json");

const conf = readJson(confPath);
const base = String(conf.version ?? "0.0.0").split("+")[0].split("-")[0];
const stamped = `${base}+${build}`;

conf.version = stamped;
writeJson(confPath, conf);

const cargo = readFileSync(cargoPath, "utf8");
const cargoNext = cargo.replace(
  /^version\s*=\s*"[^"]+"/m,
  `version = "${stamped}"`,
);
if (cargoNext === cargo) {
  console.error("Failed to update version in", cargoPath);
  process.exit(1);
}
writeFileSync(cargoPath, cargoNext);

const pkg = readJson(rootPkgPath);
pkg.version = stamped;
writeJson(rootPkgPath, pkg);

console.log(`Stamped version ${stamped}`);
