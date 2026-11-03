#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-shared-paths.mjs — every Rust file that reaches the account-wide `~/.agentmux/shared/`
// folder is named in docs/shared-paths.json, under the entry that says what it shares and how.
// Report: docs/reports/REPORT_ISOLATION_BETWEEN_INSTALLS_CHANNELS_VERSIONS_2026_11_03.md §5.
//
//   node scripts/check-shared-paths.mjs            gate (what CI runs)
//   node scripts/check-shared-paths.mjs --list     the declared entries and their users
//
// Two AgentMux installs on one machine share `shared/` on purpose (agents, history, accounts), so
// a new path there is a decision: what is it, why is it shared, how is it versioned, what keeps two
// writers apart. The gate fails when a non-test source file mentions a shared-folder resolver
// (`resolve_global_shared_root`, `resolve_shared_*`, `resolve_identity_store_path`, a `shared_dir`
// field or `.join("shared")`) and no entry in the manifest names it, when an entry is missing a
// field, and when an entry names a file that no longer does that (so the list does not rot).
// A line in a comment is not a use. Test files are not scanned.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const MANIFEST = "docs/shared-paths.json";
const USE = /resolve_global_shared_root|resolve_shared_[a-z_]+\(|resolve_identity_store_path\(|\bshared_dir\b|join\("shared"\)/;
const FIELDS = ["name", "paths", "why", "gate", "lock", "users"];

function walk(dir, out) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.name === "target" || e.name === "node_modules" || e.name.startsWith(".")) continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (e.name.endsWith(".rs")) out.push(p);
  }
}

function isTest(rel) {
  return /(_tests?\.rs|\/tests?\.rs|\/tests?\/|\/testdata\/)/.test(rel);
}

/** Whether non-comment code in `text` reaches the shared folder. */
function usesShared(text) {
  return text.split("\n").some((line) => !/^\s*\/\//.test(line) && USE.test(line));
}

function globToRegex(glob) {
  const re = glob
    .replace(/[.+^${}()|[\]\\]/g, "\\$&")
    .replace(/\*\*/g, "\u0000")
    .replace(/\*/g, "[^/]*")
    .replace(/\u0000/g, ".*");
  return new RegExp(`^${re}$`);
}

export function check(manifest, users) {
  const errors = [];
  const entries = manifest.entries ?? [];
  const claimed = new Map();
  for (const [i, e] of entries.entries()) {
    const label = e.name ? `"${e.name}"` : `entry ${i + 1}`;
    for (const f of FIELDS) {
      const v = e[f];
      if (v === undefined || v === null || (typeof v === "string" && !v.trim()) || (Array.isArray(v) && v.length === 0)) {
        errors.push(`${MANIFEST}: ${label} has no ${f}`);
      }
    }
    for (const glob of e.users ?? []) {
      const re = globToRegex(glob);
      const hits = users.filter((u) => re.test(u));
      if (hits.length === 0) errors.push(`${MANIFEST}: ${label} names ${glob}, which no longer reaches the shared folder`);
      for (const h of hits) if (!claimed.has(h)) claimed.set(h, e.name);
    }
  }
  for (const u of users) {
    if (!claimed.has(u)) {
      errors.push(`${u} reaches the shared folder and no entry in ${MANIFEST} names it: add it to the entry it belongs to, or add an entry (what it shares, why, its gate, its lock)`);
    }
  }
  return { errors, claimed };
}

function main() {
  const manifest = JSON.parse(fs.readFileSync(path.join(root, MANIFEST), "utf8"));
  const files = [];
  walk(path.join(root, "crates"), files);
  const users = files
    .map((f) => path.relative(root, f).split(path.sep).join("/"))
    .filter((rel) => !isTest(rel))
    .filter((rel) => usesShared(fs.readFileSync(path.join(root, rel), "utf8")))
    .sort();
  if (process.argv.includes("--list")) {
    for (const e of manifest.entries) {
      console.log(`${e.name}  [${e.paths.join(", ")}]\n  why: ${e.why}\n  gate: ${e.gate}\n  lock: ${e.lock}\n  users: ${e.users.join(", ")}\n`);
    }
    return;
  }
  const { errors } = check(manifest, users);
  if (errors.length) {
    console.error(errors.map((e) => `check-shared-paths: ${e}`).join("\n"));
    process.exit(1);
  }
  console.log(`check-shared-paths: ok. ${users.length} files reach the shared folder, all declared in ${manifest.entries.length} entries.`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
