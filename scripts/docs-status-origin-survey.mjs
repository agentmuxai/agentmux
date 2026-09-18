#!/usr/bin/env node
// docs-status-origin-survey.mjs — rank the reverse-check candidates by how
// likely their Status line is stale, using the commit that ADDED each doc.
//
// WHY
//
// `docs-stale-sweep.mjs --json`'s §5.6 reverse check flags a doc whose Status
// claims it was never built while source files cite it by name. That is a
// candidate list, not a verdict: deciding whether a status is true is
// judgement, and the first ten triaged by hand produced two `active`s that
// would have been wrong as `implemented`. So this does not rewrite anything.
//
// What it does is order the queue. Of those first ten, SIX shared one shape:
// the spec was added to the tree by its OWN implementing PR — which is
// `feedback_no_doc_only_prs` working exactly as intended — with a Status line
// written before that PR merged and never revised on the way past. When the
// commit that introduces a doc also touches Rust/TypeScript, the doc's
// "proposed" was describing a plan that shipped in the same commit.
//
// That signal is cheap and mechanical, and it splits 269 candidates into a
// high-yield front of the queue and a tail that needs real archaeology.
//
// Usage:
//   node scripts/docs-stale-sweep.mjs --json > sweep.json
//   node scripts/docs-status-origin-survey.mjs sweep.json
//   node scripts/docs-status-origin-survey.mjs sweep.json --json
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const args = process.argv.slice(2);
const JSON_OUT = args.includes("--json");
const input = args.find((a) => !a.startsWith("--")) || "sweep.json";

function git(...a) {
  return execFileSync("git", a, { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
}

const SOURCE_EXTS = new Set(["rs", "ts", "tsx", "mjs", "cjs", "js", "css", "sql"]);
const isSource = (p) => SOURCE_EXTS.has(p.split(".").pop()) && !p.startsWith("docs/");

const sweep = JSON.parse(readFileSync(input, "utf8"));
const candidates = sweep.claimedUnbuilt ?? [];
const wanted = new Map(candidates.map((c) => [c.doc, c]));

// Pass 1: walk every add-or-rename under the doc trees, newest first, chasing
// each candidate back through its renames to the commit that first introduced
// it.
//
// Following renames is not optional here. PR #2920 folded the top-level
// `specs/` tree into `docs/specs/` and repointed ~90 source-file citations in
// the same commit, so a naive "first A record for this exact path" reports
// that move as the origin of 60-odd specs AND sees source files in it --
// exactly the signal this script is looking for, on exactly the commits where
// it means nothing. `--follow` would handle it but takes one git process per
// path; this is one process total.
//
// `specs/` (the pre-#2920 location) is in the pathspec so the chain does not
// dead-end at the move.
const log = git(
  "log", "--diff-filter=AR", "-M", "--format=%x00%H|%ad|%s", "--date=short",
  "--name-status", "--", "docs/", "specs/",
);

// Each candidate's path *as of the commit currently being examined*. Starts as
// today's path and walks backwards as renames are encountered.
const pathNow = new Map([...wanted.keys()].map((d) => [d, d]));
const byPath = new Map([...pathNow].map(([doc, p]) => [p, doc]));
const origin = new Map(); // doc -> {sha, date, subject}

for (const chunk of log.split("\0").slice(1)) {
  const [header, ...lines] = chunk.split("\n");
  const [sha, date, ...rest] = header.split("|");
  const subject = rest.join("|");
  for (const line of lines) {
    if (!line) continue;
    const parts = line.split("\t");
    const kind = parts[0][0];
    if (kind === "R") {
      const [, from, to] = parts;
      const doc = byPath.get(to);
      if (doc === undefined || origin.has(doc)) continue;
      byPath.delete(to);
      byPath.set(from, doc);
      pathNow.set(doc, from);
    } else if (kind === "A") {
      const doc = byPath.get(parts[1]);
      if (doc === undefined || origin.has(doc)) continue;
      origin.set(doc, { sha, date, subject });
    }
  }
}

// Pass 2: for each distinct introducing commit, did it also touch source?
// `git show` takes many revisions at once, so this is a handful of processes
// instead of one per commit.
const shas = [...new Set([...origin.values()].map((o) => o.sha))];
const touchedSource = new Map();
const BATCH = 200;
for (let i = 0; i < shas.length; i += BATCH) {
  const batch = shas.slice(i, i + BATCH);
  const out = git("show", "--format=%x00%H", "--name-only", ...batch);
  for (const chunk of out.split("\0").slice(1)) {
    const [sha, ...files] = chunk.split("\n");
    touchedSource.set(sha.trim(), files.filter(isSource));
  }
}

const prOf = (subject) => (subject.match(/\(#(\d+)\)\s*$/) || [])[1] ?? null;

const rows = candidates.map((c) => {
  const o = origin.get(c.doc) ?? null;
  const src = o ? touchedSource.get(o.sha) ?? [] : [];
  return {
    doc: c.doc,
    status: c.status,
    citedBy: c.cited.length,
    origin: o && { ...o, pr: prOf(o.subject) },
    shippedWithOwnCode: src.length > 0,
    sourceFilesInOriginCommit: src.length,
  };
});

// Highest-yield first: shipped alongside its own code, then most-cited.
rows.sort((a, b) =>
  Number(b.shippedWithOwnCode) - Number(a.shippedWithOwnCode) || b.citedBy - a.citedBy);

if (JSON_OUT) {
  console.log(JSON.stringify({ generatedAt: new Date().toISOString(), rows }, null, 2));
  process.exit(0);
}

const hot = rows.filter((r) => r.shippedWithOwnCode);
console.log(`# Reverse-check candidates by origin commit\n`);
console.log(`${rows.length} candidate(s). ${hot.length} were added to the tree by a`);
console.log(`commit that also touched source -- i.e. the doc shipped with its own`);
console.log(`implementation and its Status line predates that merge.\n`);
console.log(`Still a queue, not a verdict: two of the first ten hand-checked were`);
console.log(`\`active\`, not \`implemented\`. Verify each against the code.\n`);
console.log(`| doc | status | cited by | added by | src files in that commit |`);
console.log(`|---|---|---:|---|---:|`);
for (const r of rows) {
  const o = r.origin;
  const by = o ? `${o.date} ${o.pr ? `#${o.pr}` : o.sha.slice(0, 8)}` : "_unknown_";
  console.log(
    `| \`${r.doc.replace(/^docs\//, "")}\` | ${r.status} | ${r.citedBy} | ${by} | ` +
      `${r.shippedWithOwnCode ? r.sourceFilesInOriginCommit : "—"} |`,
  );
}
