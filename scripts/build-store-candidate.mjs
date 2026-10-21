#!/usr/bin/env node
// Generates the DBX Store candidate metadata from packaged artifacts.
//
// Why a script instead of hand-editing: `candidates/<plugin-id>.json` pins each
// target with an exact SHA-256 and byte size. The signing workflow rejects any
// mismatch, so these values must come from the real build output, never by hand.
//
// Usage:
//   node scripts/build-store-candidate.mjs --repo yuzhi/dbx-plugin-opcua --tag v0.1.0
//
// Artifact sources:
//   dist/*.artifact.json          default; one entry per locally built package
//   --release-candidates <path>   the aggregated manifest downloaded from a
//                                 published GitHub Release (all platforms)
//
// The aggregated file is never auto-detected: a stale copy that does not match
// the bytes actually uploaded would pin the wrong SHA-256 and be rejected by the
// signing workflow. Pass it explicitly, and only for a release that is already
// published.
//
// Listing fields (name, description, tags, license, localizations, ...) are read
// from .dbx-store.json when present; everything else is derived from manifest.json.

import { readFileSync, writeFileSync, existsSync, readdirSync, mkdirSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function arg(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  return index !== -1 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

const repo = arg("repo", "yuzhi/dbx-plugin-opcua");
const distDir = arg("dist", join(root, "dist"));
const outArg = arg("out", "");

const manifest = JSON.parse(readFileSync(join(root, "manifest.json"), "utf8"));
const version = manifest.version;
const tag = arg("tag", `v${version}`);
const candidatePath = outArg || join(root, "store", "candidates", `${manifest.id}.json`);

// ---------------------------------------------------------------------------
// Collect targets from build output
// ---------------------------------------------------------------------------

const entries = [];
const releaseCandidates = arg("release-candidates", "");

if (releaseCandidates) {
  // Downloaded from a published GitHub Release: { plugin, artifacts: [...] }
  const rc = JSON.parse(readFileSync(releaseCandidates, "utf8"));
  for (const artifact of rc.artifacts || []) {
    entries.push({ target: artifact.target, sha256: artifact.sha256, size: artifact.size });
  }
} else if (existsSync(distDir)) {
  for (const file of readdirSync(distDir).filter((name) => name.endsWith(".artifact.json")).sort()) {
    const artifact = JSON.parse(readFileSync(join(distDir, file), "utf8"));
    entries.push({ target: artifact.target, sha256: artifact.sha256, size: artifact.size });
  }
}

if (entries.length === 0) {
  console.error(`No build artifacts found in ${distDir}. Run \`dbx-plugin package .\` first.`);
  process.exit(1);
}

const targets = entries
  .sort((left, right) => left.target.localeCompare(right.target))
  .map(({ target, sha256, size }) => ({
    target,
    url: `https://github.com/${repo}/releases/download/${tag}/${manifest.id}-${version}-${target}.dbxp`,
    sha256: sha256.toLowerCase(),
    size,
  }));

// ---------------------------------------------------------------------------
// Listing metadata (preserved across rebuilds, defaults on first run)
// ---------------------------------------------------------------------------

// Listing text comes from .dbx-store.json -- the same source the store's own
// synchronizer reads -- so the candidate and the repository metadata cannot
// drift apart. Anything absent there falls back to the manifest, then defaults.

const metaPath = join(root, ".dbx-store.json");
const meta = existsSync(metaPath) ? JSON.parse(readFileSync(metaPath, "utf8")) : {};

const allowedMeta = new Set([
  "name", "description", "icon", "tags", "permissions",
  "source", "homepage", "license", "releaseNotes", "localizations",
]);
for (const key of Object.keys(meta)) {
  if (!allowedMeta.has(key)) {
    console.error(`Unsupported .dbx-store.json field '${key}'`);
    process.exit(1);
  }
}

const defaults = {
  tags: ["opcua", "industrial", "plc", "iiot", "automation"],
  icon: `https://raw.githubusercontent.com/${repo}/${tag}/assets/plugin.svg`,
  // Must match the repository LICENSE and backend/Cargo.toml (both MIT).
  license: "MIT",
  releaseNotes: "Initial release: connect to an OPC UA server, browse the address space, read node values, and write scalar values.",
  localizations: {
    "zh-CN": {
      name: "OPC UA",
      description: "连接 OPC UA 服务器 / PLC，浏览地址空间，读取与写入节点值，并可将点位加入监视列表持续观察与就地改写下发。",
    },
  },
};

// A relative icon path resolves against the release tag.
let icon = meta.icon ?? defaults.icon;
if (icon && !/^https?:\/\//.test(icon)) {
  icon = `https://raw.githubusercontent.com/${repo}/${tag}/${icon.replace(/^\/+/, "")}`;
}

// Store URLs must be immutable or version-addressed.
function versionAddressed(url) {
  if (!url) return url;
  if (/\/(tree|releases\/tag|commit)\//.test(url)) return url;
  return `${url.replace(/\/+$/, "")}/tree/${tag}`;
}

const candidate = {
  schemaVersion: 1,
  id: manifest.id,
  publisher: manifest.publisher,
  version,
  name: meta.name ?? manifest.name,
  description: meta.description ?? manifest.description,
  icon,
  tags: meta.tags ?? defaults.tags,
  permissions: [...(meta.permissions ?? manifest.permissions ?? [])].sort(),
  source: versionAddressed(meta.source) ?? `https://github.com/${repo}/tree/${tag}`,
  homepage: meta.homepage ?? `https://github.com/${repo}`,
  license: meta.license ?? defaults.license,
  releaseNotes: meta.releaseNotes ?? defaults.releaseNotes,
  localizations: meta.localizations ?? defaults.localizations,
  targets,
};

mkdirSync(dirname(candidatePath), { recursive: true });
writeFileSync(candidatePath, `${JSON.stringify(candidate, null, 2)}\n`);

console.log(`Wrote ${candidatePath}`);
for (const target of targets) {
  console.log(`  ${target.target.padEnd(14)} ${target.sha256}  ${target.size} bytes`);
}
