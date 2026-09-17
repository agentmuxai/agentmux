#!/usr/bin/env node
// Measure docs/specs against the rules docs/specs/README.md actually states,
// and RATCHET them: the violation count may fall, never rise.
//
// The rules (docs-lifecycle hardening Phase 1, SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md):
//
//   R1  Status first word ∈ {draft,proposed,active,implemented,living,historical,superseded}
//   R2  `active`      MUST say what shipped (PR #s) and what remains
//   R3  `implemented` MUST cite the implementing PR(s)
//   R4  `superseded`  REQUIRES a Superseded-by: resolving to a real path
//
// WHY A RATCHET, AND NOT A GATE.
//
// `check-doc-status.sh` already enforces R1/R4 — but deliberately scoped to
// CHANGED FILES, because ~40% of the corpus predates the vocabulary and a
// repo-wide gate would fail every PR on pre-existing debt and be switched off
// within a day (its own comment says three previous attempts died that way).
// That choice is right, and it is also why the backlog has been flat at ~357
// for a month: it stops the debt growing and applies no pressure to shrink it.
//
// A ratchet is the missing half. It never fails a PR for debt someone else
// left, but it does fail one that ADDS to it — and the number only moves one
// way. Burn-down then happens through normal work instead of needing a project.
//
// Usage:
//   node scripts/check-docs-lifecycle.mjs              # report + ratchet check
//   node scripts/check-docs-lifecycle.mjs --write-baseline
//   node scripts/check-docs-lifecycle.mjs --list R3    # which files violate R3
import fs from "node:fs";
import path from "node:path";

const ENUM = ["draft", "proposed", "active", "implemented", "living", "historical", "superseded"];
const DIR = "docs/specs";
const BASELINE = "docs/specs/.lifecycle-baseline.json";
const PR_CITED = /#\d{3,}/;

const files = fs
    .readdirSync(DIR)
    .filter((f) => f.endsWith(".md"))
    .map((f) => path.join(DIR, f));

const v = { noStatus: [], R1: [], R2: [], R3: [], R4: [] };
const byStatus = {};

for (const f of files) {
    const src = fs.readFileSync(f, "utf8");
    const rel = f.replace(/\\/g, "/");
    const m = src.match(/^\*\*Status:\*\*\s*(.+)$/m);
    if (!m) { v.noStatus.push(rel); continue; }

    const line = m[1].trim();
    const first = line.split(/[\s—–-]+/)[0].toLowerCase().replace(/[^a-z]/g, "");
    if (!ENUM.includes(first)) { v.R1.push(`${rel}  "${line.slice(0, 60)}"`); continue; }
    byStatus[first] = (byStatus[first] || 0) + 1;

    if (first === "active" && !PR_CITED.test(line)) v.R2.push(rel);
    if (first === "implemented" && !PR_CITED.test(line)) v.R3.push(rel);
    if (first === "superseded") {
        const sb = src.match(/^\*\*Superseded-by:\*\*\s*(.+)$/m);
        const target = sb && (sb[1].match(/[\w./-]+\.md/) || [])[0];
        const resolved = target && (target.startsWith("docs/") ? target : path.join(DIR, target));
        if (!target || !fs.existsSync(resolved)) v.R4.push(`${rel} -> ${target || "(none)"}`);
    }
}

const counts = {
    noStatus: v.noStatus.length,
    R1: v.R1.length,
    R2: v.R2.length,
    R3: v.R3.length,
    R4: v.R4.length,
};
const total = Object.values(counts).reduce((a, b) => a + b, 0);

const listArg = process.argv.indexOf("--list");
if (listArg > -1) {
    const key = process.argv[listArg + 1];
    if (!v[key]) { console.error(`unknown rule "${key}" — one of: ${Object.keys(v).join(", ")}`); process.exit(2); }
    for (const x of v[key]) console.log(x);
    process.exit(0);
}

console.log(`docs/specs: ${files.length} files\n`);
console.log("Status distribution (enum-valid only):");
for (const [k, n] of Object.entries(byStatus).sort((a, b) => b[1] - a[1])) {
    console.log(`  ${String(n).padStart(4)}  ${k}`);
}
console.log("\nDeviation from docs/specs/README.md:");
console.log(`  no **Status:** line at all            ${String(counts.noStatus).padStart(4)}`);
console.log(`  R1  status outside the closed enum    ${String(counts.R1).padStart(4)}`);
console.log(`  R2  'active' citing no PR             ${String(counts.R2).padStart(4)}`);
console.log(`  R3  'implemented' citing no PR        ${String(counts.R3).padStart(4)}`);
console.log(`  R4  'superseded' ptr missing/dangling ${String(counts.R4).padStart(4)}`);
console.log(`  ${"".padStart(38, "-")}`);
console.log(`  TOTAL                                 ${String(total).padStart(4)}` +
    `  (${((total / files.length) * 100).toFixed(1)}% of files)`);

if (process.argv.includes("--write-baseline")) {
    fs.writeFileSync(BASELINE, JSON.stringify({ total, counts, files: files.length }, null, 2) + "\n");
    console.log(`\nbaseline written -> ${BASELINE}`);
    process.exit(0);
}

if (!fs.existsSync(BASELINE)) {
    console.log(`\n(no baseline at ${BASELINE}; run with --write-baseline to establish one)`);
    process.exit(0);
}

const base = JSON.parse(fs.readFileSync(BASELINE, "utf8"));
console.log(`\nRatchet: baseline ${base.total}, now ${total}.`);
if (total > base.total) {
    console.error(
        `\ncheck-docs-lifecycle: FAILED — lifecycle violations rose by ${total - base.total}.\n\n` +
        `This gate never asks you to fix debt you did not create. It only asks that a\n` +
        `change not ADD to it. Either give the doc(s) you touched a compliant Status\n` +
        `line (see docs/specs/README.md), or if the rise is legitimate, re-baseline\n` +
        `with: node scripts/check-docs-lifecycle.mjs --write-baseline\n\n` +
        `Rules that moved: ` +
        Object.entries(counts)
            .filter(([k, n]) => n !== base.counts[k])
            .map(([k, n]) => `${k} ${base.counts[k]} -> ${n}`)
            .join(", ") +
        `\nList the offenders with: node scripts/check-docs-lifecycle.mjs --list R1\n`
    );
    process.exit(1);
}
if (total < base.total) {
    console.log(`  ${base.total - total} fewer than baseline. Re-baseline to lock the gain in:`);
    console.log("  node scripts/check-docs-lifecycle.mjs --write-baseline");
}
console.log("check-docs-lifecycle: ok");
