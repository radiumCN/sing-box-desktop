/**
 * Keeps src-tauri/Cargo.toml's [package] version in sync with package.json.
 *
 * package.json is the single source of truth for the app version:
 *   - tauri.conf.json already reads it via "version": "../package.json"
 *     (drives the bundled app version + the frontend's getVersion()).
 *   - the Rust runtime reads the same value via app_handle.package_info().version.
 *   - Cargo.toml only needs it for crate metadata / user-agent strings, so this
 *     script copies it over instead of requiring a second manual edit per release.
 *
 * Runs automatically before each build via tauri.conf.json's beforeBuildCommand,
 * and can be invoked directly with `npm run sync-version`.
 */

import { readFileSync, writeFileSync } from "fs";
import { dirname } from "path";
import { fileURLToPath } from "url";

const root = `${dirname(fileURLToPath(import.meta.url))}/..`;
const cargoPath = `${root}/src-tauri/Cargo.toml`;

const version = JSON.parse(readFileSync(`${root}/package.json`, "utf8")).version;
if (!version) {
  console.error("[sync-version] package.json has no version field");
  process.exit(1);
}

const cargo = readFileSync(cargoPath, "utf8");

// Replace the version only inside the [package] section, leaving dependency
// versions elsewhere in the file untouched.
const packageVersionRe = /(\[package\][^[]*?\nversion\s*=\s*")([^"]*)(")/;
const match = cargo.match(packageVersionRe);
if (!match) {
  console.error("[sync-version] could not find [package] version in Cargo.toml");
  process.exit(1);
}

const current = match[2];
if (current === version) {
  console.log(`[sync-version] Cargo.toml already at ${version}`);
  process.exit(0);
}

writeFileSync(cargoPath, cargo.replace(packageVersionRe, `$1${version}$3`));
console.log(`[sync-version] Cargo.toml ${current} → ${version}`);
