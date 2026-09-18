#!/usr/bin/env node
// check-doc-links.mjs — CI gate: a relative Markdown link between docs must resolve.
//
// docs/specs/README.md ("a broken pointer is worse than none")
// docs/specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md (Phase 2 / Part 4)
//
// THE RULE: if a Markdown link in a tracked doc points at a relative path, that
// path must exist when resolved against the directory of the file containing it.
//
// This is the sibling of check-spec-citations.sh. That one catches a *cited
// path* naming `docs/specs/<x>.md` from anywhere (including source comments);
// this one catches a *Markdown link* whose target is wrong relative to where it
// was written. The two miss opposite things:
//
//   docs/reports/X.md containing `](specs/archive/Y.md)`
//     -> check-spec-citations.sh: does not fire. The text does not contain the
//        substring `docs/specs/`, so it is not recognised as a spec citation.
//     -> this gate: fires. Resolved from docs/reports/ the target would be
//        docs/reports/specs/archive/Y.md, which does not exist.
//
// That exact shape is why this exists: the top-level `specs/` tree was folded
// into `docs/specs/` (PR #2920), and links written as `specs/archive/...` kept
// working only from files that happened to sit in `docs/`. From anywhere else
// they have been silently dead ever since. The first run over the whole tree
// found 45 broken links across 27 files.
//
// SCOPED TO CHANGED FILES by default, for the reason every other docs gate here
// is: a repo-wide gate fails every PR on somebody else's rot and gets switched
// off within a day -- the documented fate of three previous attempts. Run with
// --all to measure the backlog instead of gating on it.
//
// Usage:
//   node scripts/check-doc-links.mjs              # changed vs origin/main
//   node scripts/check-doc-links.mjs --all        # every tracked doc (report)
//   node scripts/check-doc-links.mjs FILE...      # explicit files
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
const ALL = args.includes("--all");
const explicit = args.filter((a) => !a.startsWith("--"));

function git(...a) {
  return execFileSync("git", a, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
}

function changedFiles() {
  const base = process.env.GITHUB_BASE_REF || "main";
  let ref = `origin/${base}`;
  try {
    git("rev-parse", "--verify", "--quiet", ref);
  } catch {
    ref = base;
  }
  let mergeBase;
  try {
    mergeBase = git("merge-base", ref, "HEAD").trim();
  } catch {
    return [];
  }
  return git("diff", "--name-only", "--diff-filter=ACMR", `${mergeBase}...HEAD`)
    .split("\n")
    .filter(Boolean);
}

const tracked = new Set(git("ls-files").split("\n").filter(Boolean));

let targets;
if (explicit.length) targets = explicit;
else if (ALL) targets = [...tracked];
else targets = changedFiles();

targets = targets.filter(
  (f) => f.endsWith(".md") && f.startsWith("docs/") && tracked.has(f),
);

// Inline links `](target)` and reference definitions `[id]: target`.
const INLINE = /\]\(\s*([^)\s]+?)(?:\s+"[^"]*")?\s*\)/g;
const REFDEF = /^\[[^\]]+\]:\s*(\S+)/gm;

// Extensions this repo actually links between. A target with no slash AND no
// known extension is prose that happens to sit in parens after a bracket --
// `](true)`, `](url)`, `](next)` all appear in real specs and are not links.
// Matching them would make this gate fire on files nobody can fix, which is how
// the three previous docs-enforcement attempts here died.
const LINKABLE = /\.(md|markdown|rs|ts|tsx|js|mjs|json|sh|yml|yaml|toml|png|svg|txt)$/i;

function isSkippable(t) {
  if (!t) return true;
  if (/^[a-z][a-z0-9+.-]*:/i.test(t)) return true; // scheme: http:, mailto:
  if (t.startsWith("#") || t.startsWith("//")) return true;
  // Site-absolute (`/internals/platform-support/`) targets the published docs
  // site, not this tree -- resolving it against the repo root is meaningless.
  if (t.startsWith("/")) return true;
  if (t.startsWith(":")) return true; // `](:GetText().Length)`
  if (t.includes("<") || t.includes("*") || t.includes("{")) return true;
  // Not a path and not a known file type -> prose, not a link.
  if (!t.includes("/") && !LINKABLE.test(t)) return true;
  return false;
}

// A link that resolves outside the repo root is a machine-local reference
// (agent memory dirs, sibling checkouts). Unresolvable here by construction,
// and not this repo's to fix -- counted separately, never gated on.
function escapesRepo(resolved) {
  return resolved.startsWith("..");
}

// Drop inline code spans. Specs here routinely *show* link syntax while
// explaining these gates -- this file does it a few lines above -- and a
// backticked example is documentation, not a pointer to resolve.
function stripInlineCode(line) {
  const parts = line.split("`");
  // Even indices are outside code spans; odd indices are inside them.
  return parts.filter((_, i) => i % 2 === 0).join(" ");
}

// Drop fenced code blocks as well. A `](...)` inside a fence is not a link -- this
// repo's specs contain ASCII-art UI mockups that *depict* a rendered Markdown
// list, and "fixing" those both breaks the box drawing and is wrong: the text
// is showing what a link looks like, not making one.
function stripFences(src) {
  const FENCE = "```";
  let inFence = false;
  return src
    .split("\n")
    .filter((line) => {
      if (line.trimStart().startsWith(FENCE)) {
        inFence = !inFence;
        return false;
      }
      return !inFence;
    })
    .map(stripInlineCode)
    .join("\n");
}

let broken = 0;
let outside = 0;
let checked = 0;
const byFile = new Map();

for (const file of targets) {
  // Read from the working tree, not `git show`: one child process per file
  // makes a whole-repo --all run take minutes. CI checks the branch out, so
  // the working tree is the thing being gated either way.
  let src;
  try {
    src = readFileSync(file, "utf8");
  } catch {
    continue; // listed in the diff but deleted
  }
  src = stripFences(src);

  const dir = path.posix.dirname(file);
  const seen = new Set();

  for (const re of [INLINE, REFDEF]) {
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(src)) !== null) {
      let target = m[1].trim();
      if (isSkippable(target)) continue;
      target = target.split("#")[0]; // drop anchor
      if (!target) continue;
      if (seen.has(target)) continue;
      seen.add(target);
      checked++;

      const resolved = path.posix.normalize(path.posix.join(dir, target));
      // A link may legitimately point at a directory or a non-md file.
      if (existsSync(resolved)) continue;
      if (escapesRepo(resolved)) {
        outside++;
        continue;
      }

      broken++;
      if (!byFile.has(file)) byFile.set(file, []);
      byFile.get(file).push({ target, resolved });
    }
  }
}

if (byFile.size) {
  console.error("");
  console.error("Broken relative Markdown links:");
  console.error("");
  for (const [file, items] of [...byFile].sort()) {
    console.error(`  ${file}`);
    for (const { target, resolved } of items) {
      console.error(`      ](${target})  ->  ${resolved}  (missing)`);
    }
  }
  console.error("");
  console.error("Fix the path, or -- if the target is genuinely gone -- de-link it");
  console.error("and keep the name as text. A broken pointer is worse than none.");
  console.error("");
}

console.error(
  `check-doc-links: ${checked} link(s) checked across ${targets.length} doc(s), ` +
    `${broken} broken in ${byFile.size} file(s), ${outside} outside-repo (ignored)`,
);

// Reporting mode never fails -- it exists to measure the backlog, not gate on it.
process.exit(ALL ? 0 : broken ? 1 : 0);
