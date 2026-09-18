#!/usr/bin/env node
// Measure docs/specs against the rules docs/specs/README.md actually states,
// and ratchet the files a branch TOUCHES so they cannot get worse.
//
// The rules (docs-lifecycle hardening Phase 1, SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md):
//
//   R1  Status first word ∈ the closed enum below (kept in step with
//       check-doc-status.sh's VALID — see the comment on ENUM)
//   R2  `active`      MUST say what shipped (PR #s) and what remains
//   R3  `implemented` MUST cite the implementing PR(s)
//   R4  `superseded`  REQUIRES a Superseded-by: resolving to a real path
//
// WHY A RATCHET, AND WHY SCOPED TO CHANGED FILES.
//
// `check-doc-status.sh` already enforces R1/R4, deliberately scoped to changed
// files: ~40% of the corpus predates the vocabulary, and a repo-wide gate would
// fail every PR on pre-existing debt and be switched off within a day (its own
// comment says three earlier attempts died exactly that way). That choice is
// right, and it is also why the backlog sat flat for a month — it stops the debt
// growing and applies no pressure to shrink it.
//
// This adds the missing half. Two earlier designs of it were both wrong:
//
//   1. A committed baseline NUMBER. It cannot tell "this branch added
//      violations" from "main gained violations while this branch was open".
//      Several agents merge spec-touching PRs a day here, so it promptly failed
//      two of my own PRs for debt that landed on main after the number was
//      written — precisely the behaviour this header promises it will not have.
//
//   2. Comparing TOTALS against the merge-base. Correct attribution, but a
//      burn-down branch earns slack: a branch that removed 327 violations could
//      add a brand-new non-compliant spec and still pass. Verified — it did.
//
// Scoping to the files the branch changed fixes both. Main's drift is excluded
// because those files are untouched, and slack is impossible because each
// touched file is compared against its own former self.
//
// Usage:
//   node scripts/check-docs-lifecycle.mjs                    # report + ratchet
//   node scripts/check-docs-lifecycle.mjs --list R3          # offenders per rule
//   node scripts/check-docs-lifecycle.mjs --since <ref>      # measure a git ref
import fs from "node:fs";
import path from "node:path";
import { execSync } from "node:child_process";

// Must stay in step with check-doc-status.sh's VALID. `retro` and `analysis`
// were added to that gate in #3378 and not to this one, so for a week any doc
// honestly marked with either failed R1 here while passing there — two gates
// disagreeing about the same vocabulary, which is worse than one gate being
// wrong. Found by this branch marking SPEC_BROWSER_PANE_LIFECYCLE `analysis`.
const ENUM = [
    "draft", "proposed", "active", "implemented",
    "living", "historical", "superseded", "retro", "analysis",
];
const DIR = "docs/specs";
const PR_CITED = /#\d{3,}/;
// Archived specs are finished by definition; holding them to "implemented MUST
// cite a PR" would manufacture a backlog nobody should burn down. docs-stale-
// sweep already excludes archive/ for the same reason. Stated once and applied
// to BOTH the working-tree and --since paths, because when only one of them
// excluded a subtree the gate silently stopped protecting it (reagent P1, #3349).
const EXCLUDED = /(^|\/)docs\/specs\/archive\//;

const sh = (c, opts) => {
    try { return execSync(c, { encoding: "utf8", stdio: ["pipe", "pipe", "ignore"], maxBuffer: 512e6, ...opts }); }
    catch { return null; }
};

const REF = (() => { const i = process.argv.indexOf("--since"); return i > -1 ? process.argv[i + 1] : null; })();
const QUIET = REF !== null || process.argv.includes("--json-violations");

// Reading a ref one `git show` at a time is ~900 spawns and takes minutes on
// Windows. `ls-tree` gives every path AND blob sha in one call; `cat-file
// --batch` streams every blob through ONE process.
let files;
let blobs = null;
if (REF) {
    const tree = (sh(`git ls-tree -r ${REF} -- ${DIR}`) || "")
        .split("\n").map((l) => l.trim()).filter(Boolean)
        .map((l) => { const m = l.match(/^\S+\s+blob\s+(\S+)\t(.+)$/); return m ? { sha: m[1], path: m[2] } : null; })
        .filter((x) => x && x.path.endsWith(".md") && !EXCLUDED.test(x.path));
    files = tree.map((t) => t.path);
    // Buffer, deliberately: cat-file reports sizes in BYTES. Slicing a decoded
    // string by them desynchronises at the first em-dash, and this corpus is
    // full of them — that bug made every file read as garbage (971 of 971).
    const raw = execSync("git cat-file --batch", {
        input: tree.map((t) => t.sha).join("\n") + "\n",
        maxBuffer: 512e6,
    });
    blobs = new Map();
    let off = 0;
    for (const t of tree) {
        const nl = raw.indexOf(0x0a, off);
        const size = Number(raw.subarray(off, nl).toString("utf8").split(" ")[2]);
        blobs.set(t.path, raw.subarray(nl + 1, nl + 1 + size).toString("utf8"));
        off = nl + 1 + size + 1;
    }
} else {
    const walk = (d, out = []) => {
        for (const e of fs.readdirSync(d, { withFileTypes: true })) {
            const full = path.join(d, e.name);
            if (e.isDirectory()) walk(full, out);
            else if (e.name.endsWith(".md")) out.push(full);
        }
        return out;
    };
    files = walk(DIR).filter((f) => !EXCLUDED.test(f.replace(/\\/g, "/")));
}
const readFile = (f) => (REF ? (blobs.get(f.replace(/\\/g, "/")) ?? null) : fs.readFileSync(f, "utf8"));

const v = { noStatus: [], R1: [], R2: [], R3: [], R4: [] };
const byStatus = {};

for (const f of files) {
    const src = readFile(f);
    if (src == null) continue;
    const rel = f.replace(/\\/g, "/");
    // Read the WHOLE Status value, not just its first physical line. A Status
    // that wraps carries its evidence (PR numbers, what shipped, what remains)
    // on continuation lines as often as the first, and a first-line-only regex
    // both under-counts compliance AND tempts an editing pass into appending
    // mid-sentence. Both bugs happened here before this was fixed.
    const lines = src.split("\n");
    const si = lines.findIndex((l) => /^\*\*Status:\*\*/.test(l));
    if (si < 0) { v.noStatus.push(rel); continue; }
    const isCont = (l) =>
        l !== undefined && l.trim() !== "" && !/^\*\*[A-Z]/.test(l) && !/^[#>|]/.test(l) && !/^\s*[-*]\s/.test(l);
    let se = si;
    while (isCont(lines[se + 1])) se++;
    const line = lines.slice(si, se + 1).join(" ").replace(/^\*\*Status:\*\*\s*/, "").trim();
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

const counts = { noStatus: v.noStatus.length, R1: v.R1.length, R2: v.R2.length, R3: v.R3.length, R4: v.R4.length };
const total = Object.values(counts).reduce((a, b) => a + b, 0);
const offenders = () => new Set(
    [...v.R1, ...v.R2, ...v.R3, ...v.R4, ...v.noStatus].map((x) => String(x).split("  ")[0].split(" -> ")[0])
);

const listArg = process.argv.indexOf("--list");
if (listArg > -1) {
    const key = process.argv[listArg + 1];
    if (!v[key]) { console.error(`unknown rule "${key}" — one of: ${Object.keys(v).join(", ")}`); process.exit(2); }
    for (const x of v[key]) console.log(x);
    process.exit(0);
}
if (process.argv.includes("--json-violations")) { console.log(JSON.stringify([...offenders()])); process.exit(0); }

if (!QUIET) {
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
}
if (REF) process.exit(0);

const mb = sh("git merge-base origin/main HEAD") || sh("git merge-base main HEAD");
if (!mb) { console.log("\n(no merge-base with main available; skipping the ratchet)"); process.exit(0); }
const baseRef = mb.trim();
const touched = (sh(`git diff --name-only ${baseRef}...HEAD -- ${DIR}`) || "")
    .split("\n").map((x) => x.trim()).filter((x) => x.endsWith(".md") && !EXCLUDED.test(x));
if (!touched.length) { console.log("\nRatchet: this branch touches no specs. ok"); process.exit(0); }

const nowBad = offenders();
const baseJson = sh(`node "${process.argv[1]}" --since ${baseRef} --json-violations`);
const baseBad = new Set(baseJson ? JSON.parse(baseJson) : []);
const added = touched.filter((f) => nowBad.has(f) && !baseBad.has(f));
const fixed = touched.filter((f) => !nowBad.has(f) && baseBad.has(f));

console.log(`\nRatchet (scoped to ${touched.length} spec(s) this branch touched):`);
console.log(`  fixed: ${fixed.length}   newly violating: ${added.length}`);
if (added.length) {
    console.error(
        `\ncheck-docs-lifecycle: FAILED — ${added.length} spec(s) this branch touched are\n` +
        `newly non-compliant:\n  ` + added.join("\n  ") + "\n\n" +
        `This gate is scoped to the files you changed: debt that landed on main since\n` +
        `you branched is not your problem, and neither is pre-existing debt in files\n` +
        `you did not touch. Give these a compliant Status line — docs/specs/README.md.\n`
    );
    process.exit(1);
}
console.log("check-docs-lifecycle: ok");
