#!/usr/bin/env node
// check-docs-dir-map.mjs — CI gate: docs/README.md's directory table must match
// the directories that actually exist.
//
// docs/specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md (Part 1.1, Phase 2)
// docs/README.md (the table this checks)
//
// THE RULE: every `dir/` named in docs/README.md's table must contain tracked
// files, and every directory directly under docs/ that contains tracked files
// must be named in the table.
//
// WHY: docs/README.md is the map — it is what tells a reader (or an agent)
// where a given kind of doc belongs. A wrong map is worse than no map, and this
// one has gone stale at least twice:
//
//   * The 2026-08-03 lifecycle audit found "this directory list and the
//     specs-location claim above were themselves found stale". docs/README.md
//     still carries that note, about itself.
//   * `docs/handoff/` was deleted by PR #2407 — a *docs cleanup sweep*. The
//     sweep removed the directory and did not update the table, so the map
//     advertised a directory that had not existed for six weeks. Meanwhile
//     `docs/debug/` was added and never listed at all.
//
// Both drifts are mechanical, and both were invisible until something looked.
// That is the failure mode the hardening spec exists to stop: one-shot fixes
// rot, self-verifying systems don't.
//
// NOT SCOPED TO CHANGED FILES, unlike the other docs gates. Those are scoped
// because their backlog runs to the hundreds and a repo-wide gate would fail
// every PR on somebody else's rot — the documented fate of three previous
// attempts. This one's backlog is exactly two, both fixed in the commit that
// adds it, and it reads a single file. So it can be absolute without ever
// failing a PR that did not cause the problem.
//
// Usage:
//   node scripts/check-docs-dir-map.mjs
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const README = "docs/README.md";

const tracked = execFileSync("git", ["ls-files", "docs/"], {
  encoding: "utf8",
  maxBuffer: 64 * 1024 * 1024,
})
  .split("\n")
  .filter(Boolean);

// Directories directly under docs/ that hold at least one tracked file.
const actual = new Set();
for (const p of tracked) {
  const parts = p.split("/");
  if (parts.length > 2) actual.add(parts[1]);
}

// Names the table claims, written as `name/` in a backticked cell. Some rows
// group several (`incident/`, `recovery/`), so scan every occurrence rather
// than assuming one per row.
//
// TABLE ROWS ONLY. The prose below the table mentions `docs-internal/`
// precisely to say it does not exist; reading that as a claim would make this
// gate demand the repo create it.
const DIR_CELL = /`([A-Za-z0-9._-]+)\/`/g;
const claimed = new Set();
for (const line of readFileSync(README, "utf8").split("\n")) {
  if (!line.trimStart().startsWith("|")) continue;
  for (const m of line.matchAll(DIR_CELL)) claimed.add(m[1]);
}

const missing = [...claimed].filter((d) => !actual.has(d)).sort();
const unlisted = [...actual].filter((d) => !claimed.has(d)).sort();

if (missing.length || unlisted.length) {
  console.error("");
  console.error(`${README} does not match the directories on disk:`);
  console.error("");
  for (const d of missing) {
    console.error(`  claimed but empty/absent   docs/${d}/`);
  }
  for (const d of unlisted) {
    console.error(`  exists but not in the map  docs/${d}/`);
  }
  console.error("");
  console.error("Update the table in docs/README.md. If a directory was removed,");
  console.error("drop its row; if one was added, say what belongs in it.");
  console.error("");
  process.exit(1);
}

console.error(
  `check-docs-dir-map: ok (${actual.size} directories, all listed and all real)`,
);
