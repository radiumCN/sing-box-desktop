/**
 * Downloads the sing-box kernel for the CURRENT HOST and drops it into
 * `src-tauri/binaries/` using Tauri's sidecar naming (`sing-box-<target-triple>[.exe]`),
 * so that `tauri dev` / `tauri build` can bundle it as an `externalBin`.
 *
 * This is only for LOCAL builds. CI (.github/workflows/release.yml) fetches the binary
 * per build target itself (it must, because macOS Intel is cross-compiled on an arm64
 * runner, where the host triple ≠ the target triple).
 *
 * Usage:
 *   node scripts/fetch-singbox.js              # latest stable, skip if already present
 *   SING_BOX_VERSION=1.11.4 node scripts/...   # pin a version
 *   FORCE=1 node scripts/fetch-singbox.js      # re-download even if present
 */

import { mkdirSync, existsSync, rmSync, renameSync, writeFileSync } from "fs";
import { execFileSync } from "child_process";
import { dirname, join } from "path";
import { fileURLToPath } from "url";
import { tmpdir } from "os";

const __dirname = dirname(fileURLToPath(import.meta.url));
const binDir = join(__dirname, "..", "src-tauri", "binaries");

// Map the host (Node) platform/arch onto: the Rust target triple Tauri expects in the
// sidecar file name, plus the OS/arch keywords sing-box uses in its release asset names.
const PLATFORMS = {
  "win32-x64":   { triple: "x86_64-pc-windows-msvc",    os: "windows", arch: "amd64", ext: "zip",    exe: ".exe" },
  "darwin-x64":  { triple: "x86_64-apple-darwin",       os: "darwin",  arch: "amd64", ext: "tar.gz", exe: "" },
  "darwin-arm64":{ triple: "aarch64-apple-darwin",      os: "darwin",  arch: "arm64", ext: "tar.gz", exe: "" },
  "linux-x64":   { triple: "x86_64-unknown-linux-gnu",  os: "linux",   arch: "amd64", ext: "tar.gz", exe: "" },
  "linux-arm64": { triple: "aarch64-unknown-linux-gnu", os: "linux",   arch: "arm64", ext: "tar.gz", exe: "" },
};

async function main() {
  const key = `${process.platform}-${process.arch}`;
  const p = PLATFORMS[key];
  if (!p) {
    console.error(`[fetch-singbox] unsupported host platform: ${key}`);
    process.exit(1);
  }

  mkdirSync(binDir, { recursive: true });
  const dest = join(binDir, `sing-box-${p.triple}${p.exe}`);
  if (existsSync(dest) && !process.env.FORCE) {
    console.log(`[fetch-singbox] already present: ${dest} (set FORCE=1 to re-download)`);
    return;
  }

  // Resolve the version: explicit pin, or the latest stable GitHub release.
  let version = process.env.SING_BOX_VERSION;
  if (!version) {
    const res = await fetch("https://api.github.com/repos/SagerNet/sing-box/releases/latest", {
      headers: { "User-Agent": "skylark-build" },
    });
    if (!res.ok) throw new Error(`GitHub API ${res.status} while resolving latest sing-box`);
    version = (await res.json()).tag_name.replace(/^v/, "");
  }

  const dirName = `sing-box-${version}-${p.os}-${p.arch}`;
  const url = `https://github.com/SagerNet/sing-box/releases/download/v${version}/${dirName}.${p.ext}`;
  console.log(`[fetch-singbox] downloading ${url}`);

  const res = await fetch(url, { headers: { "User-Agent": "skylark-build" } });
  if (!res.ok) throw new Error(`download failed: HTTP ${res.status} for ${url}`);
  const archive = join(tmpdir(), `singbox.${p.ext}`);
  writeFileSync(archive, Buffer.from(await res.arrayBuffer()));

  // The host `tar` is bsdtar (Windows 10+, macOS) or GNU tar (Linux); both extract the
  // archive format we use on that very host (zip on Windows, tar.gz elsewhere).
  const member = `${dirName}/sing-box${p.exe}`;
  execFileSync("tar", ["-xf", archive, "--strip-components=1", "-C", binDir, member], { stdio: "inherit" });
  rmSync(archive, { force: true });

  renameSync(join(binDir, `sing-box${p.exe}`), dest);
  if (!p.exe) {
    // chmod +x on Unix so Tauri bundles an executable sidecar.
    execFileSync("chmod", ["+x", dest]);
  }
  console.log(`[fetch-singbox] ready: ${dest}`);
}

main().catch((e) => {
  console.error(`[fetch-singbox] ${e.message}`);
  process.exit(1);
});
