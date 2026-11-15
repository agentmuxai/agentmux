#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// gen-docs-index.mjs — regenerate the machine-maintained half of
// docs/specs/INDEX.md.
//
// docs/specs/SPEC_DOCS_INDEX_GENERATOR_NODE_PORT_2026_09_23.md (this port)
// docs/specs/PLAN_DOCS_CLEANUP_EXECUTION_2026_09_01.md (Batch D)
// docs/specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md (Phase 3)
//
// Usage (the documented entry point is the wrapper, which just execs this):
//   bash scripts/gen-docs-index.sh              # rewrite the generated section
//   bash scripts/gen-docs-index.sh --check      # CI: assert, but only when
//                                               # this branch touches docs/specs
//   bash scripts/gen-docs-index.sh --check-all  # assert against the whole tree
//   node scripts/gen-docs-index.mjs [same flags]
//
// Run from the repository root.
//
// WHY NODE (2026-09-23)
//
// This was a bash script. On Windows it took ~200 s per run for 956 specs: the
// per-file loop started about ten processes per spec, and Git Bash emulates
// fork(), so each start cost ~20 ms. It also needed bash >= 4 (`declare -A`),
// which stock macOS does not ship, and its output depended on platform tool
// behaviour (grep strips CR on Windows only). One Node process does the same
// work in a fraction of a second on every OS, and CI now runs it on all three.
// Output is byte-identical to the shell version for every tree without CRLF
// or non-ASCII spec filenames; the spec's §6 lists the deliberate differences.
//
// WHY GENERATED, AND HOW IT IS GROUPED (changed 2026-11-14)
//
// The index used to group by Status. A reader looking for "the spec about X" then
// had to scan 1,300 rows by title, and a replaced spec looked as live as the one
// that replaced it (a history-compaction spec was nearly rewritten from scratch
// because the built one was hard to find, task #47664). It now groups by AREA, and
// each row carries its Status, its date and what it replaces or is replaced by.
// A spec says its area, and what it replaces, in its first 40 lines:
//
//   **Area:** history store
//   **Supersedes:** SPEC_OLD_2026_09_01          (optional; names a spec in docs/specs)
//   **Superseded-by:** SPEC_NEW_2026_10_21       (optional; the Status rules require it for `superseded`)
//
// Either side of a replacement is enough: the index shows both directions. A spec
// with no Area line lands in "unsorted" with its date, newest first, so nothing is
// hidden and nothing needs a bulk restamp.
//
// The older reasoning, kept for what it says about Status:
//
// The curated index above the marker is genuinely useful and stays hand-written:
// it answers "where do I start for subsystem X", which no generator can.
//
// What it cannot do is keep up. Measured 2026-09-01: it lists 77 of 730 specs,
// and 165 specs added in the previous 30 days are absent from it. Nothing was
// broken — every curated entry still resolves — it is simply being outrun at
// ~165 new docs/month. That is the failure Phase 3 named: "the tool that's
// supposed to help an agent find the current doc is itself an instance of the
// problem it's meant to solve."
//
// Grouping by Status rather than subsystem is deliberate. Subsystem grouping
// needs human judgement (the 308 distinct filename prefixes do not map to
// subsystems), so a generator doing it would produce noise. Status answers the
// question a reader actually arrives with — "is this real, or someone's idea?" —
// and it is now an enforced closed enum (scripts/check-doc-status.sh), so it can
// be trusted enough to sort on.
//
// MERGE CONFLICT IN INDEX.md? Do not hand-resolve it.
//
//   git checkout --ours docs/specs/INDEX.md && bash scripts/gen-docs-index.sh
//
// Either side is equally wrong once both branches have added specs, and the
// generated content is a pure function of the tree — so regenerating after
// taking either side is always correct, and hand-merging never is. This is a
// committed generated file, so any two PRs that add a spec conflict here; the
// conflict is expected and costs one command.
//
// Two failure modes worth knowing, both hit while building the shell version:
//
//   - CI tests the PR MERGE commit, not your branch head. If --check passes
//     locally and fails on CI, your branch is stale: git will happily merge a
//     new spec's ROW in from main while keeping YOUR copy of the rest, so the
//     merged file can be internally inconsistent in a way neither parent was.
//     Merge main, regenerate. The "N specs" line printed on every run is the
//     fastest way to spot it — a count differing from yours means exactly this.
//   - Verifying with a `(?<!...)` lookbehind in ripgrep silently finds nothing:
//     its default engine rejects lookaround, and a redirected stderr turns that
//     parse error into a confident "0 matches".
//
// BYTES, NOT TEXT
//
// Byte-identical output on every machine, or --check is a coin flip: it compares
// a committed file against a fresh run, so ANY environment-dependent behaviour
// makes CI disagree with the developer who just regenerated. The shell version
// pinned LC_ALL=C for this. The equivalent here: every file is read as latin1
// (one character per byte), matching uses explicit ASCII classes (never `\s`,
// which also matches U+00A0), lowercasing is ASCII-only, sorting compares bytes,
// and the result is written back as latin1 so UTF-8 in titles round-trips
// unchanged. Constant text in this file is UTF-8 and is converted with `bytes()`
// before it meets file content.

import { execFileSync } from "node:child_process";
import {
    closeSync,
    existsSync,
    openSync,
    readFileSync,
    readSync,
    readdirSync,
    renameSync,
    rmSync,
    statSync,
    writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { isMain } from "./lib/is-main.mjs";

export const INDEX = "docs/specs/INDEX.md";
export const BEGIN = "<!-- BEGIN GENERATED INDEX — edit scripts/gen-docs-index.sh, not this section -->";
export const END = "<!-- END GENERATED INDEX -->";

/** Canonical buckets, in output order. */
export const CANONICAL = ["implemented", "active", "proposed", "draft", "living", "historical", "superseded"];
const NONE = "__none__";
/** The group of specs that name no area. */
export const UNSORTED = "";

/** Lines of each spec that are examined (the shell version's `head -40`). */
const HEAD_LINES = 40;

/** UTF-8 source text → the latin1 "byte string" every file-derived value uses. */
export function bytes(s) {
    return Buffer.from(s, "utf8").toString("latin1");
}

/** A latin1 byte string → a Buffer holding exactly those bytes. */
function toBuffer(s) {
    return Buffer.from(s, "latin1");
}

/** `tolower` under LC_ALL=C: A–Z only. */
export function asciiLower(s) {
    return s.replace(/[A-Z]/g, (c) => String.fromCharCode(c.charCodeAt(0) + 32));
}

/** Byte-order comparison (what `sort` does under LC_ALL=C). Inputs are latin1
 *  byte strings, so comparing UTF-16 code units compares bytes. */
export function byteCompare(a, b) {
    return a < b ? -1 : a > b ? 1 : 0;
}

/** The first HEAD_LINES lines of a latin1 text, as an array. A trailing
 *  newline does not create an extra line, as with `head`/`grep`. */
function headLines(text) {
    const lines = text.split("\n", HEAD_LINES + 1).slice(0, HEAD_LINES);
    return lines;
}

// The Status bucket of a spec, from its first 40 lines — the shell version's
//
//   grep -m1 -i '^\*\*Status:\*\*' | sed 's/^\*\*Status:\*\*[[:space:]]*//I'
//     | awk '{print tolower($1)}' | sed 's/[^a-z]//g'
//
// under LC_ALL=C. First matching line (ASCII case-insensitive); strip the
// prefix and any [[:space:]]; first field split on blanks/newlines as awk's
// default FS does; ASCII-lowercase; keep only a–z. No Status line, or nothing
// left after that, is `__none__`.
//
// A trailing CR is dropped first (spec §6.2); it would be deleted by the
// `[^a-z]` pass anyway, so the bucket is unaffected.
export function statusOf(text) {
    for (const raw of headLines(text)) {
        const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
        if (!/^\*\*status:\*\*/i.test(line)) continue;
        const rest = line.replace(/^\*\*status:\*\*[ \t\n\v\f\r]*/i, "");
        const first = rest.replace(/^[ \t\n]+/, "").split(/[ \t\n]+/)[0] ?? "";
        const word = asciiLower(first).replace(/[^a-z]/g, "");
        return word === "" ? NONE : word;
    }
    return NONE;
}

/**
 * The display title: first line starting with "# " in the first 40 lines, with
 * the "#" and following spaces removed and every "|" deleted (it would break
 * the table). Empty or absent falls back to the filename.
 *
 * A trailing CR is dropped (spec §6.2). The shell version kept it on Linux and
 * macOS and dropped it on Windows (grep's text I/O there reads CRLF as LF).
 */
export function titleOf(text, fallback) {
    for (const raw of headLines(text)) {
        if (!raw.startsWith("# ")) continue;
        const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
        const title = line.replace(/^# */, "").replace(/\|/g, "");
        return title === "" ? fallback : title;
    }
    return fallback;
}

/** The value of the first `**<name>:**` line in a spec's first 40 lines (ASCII
 *  case-insensitive), or `null`. A trailing CR and blanks are dropped. */
function fieldOf(text, name) {
    const prefix = new RegExp(`^\\*\\*${name}:\\*\\*[ \t\n\v\f\r]*`, "i");
    for (const raw of headLines(text)) {
        const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
        if (prefix.test(line)) return line.replace(prefix, "").replace(/[ \t\v\f]+$/, "");
    }
    return null;
}

/** A name that can head a group: short, plain words. An `**Area:**` line that
 *  lists code paths or links (some older specs use it that way) is not one. */
const AREA_NAME = /^[a-z0-9][a-z0-9 /&+-]{0,38}$/;

/** The area a spec says it belongs to (`**Area:** history store`): ASCII-lowercased
 *  and blanks collapsed. `""` when it has none, or says something that is not a
 *  short plain name, which is "unsorted". */
export function areaOf(text) {
    const value = fieldOf(text, "area");
    if (value === null) return UNSORTED;
    const name = asciiLower(value).replace(/[ \t]+/g, " ").trim();
    return AREA_NAME.test(name) ? name : UNSORTED;
}

/** The spec names in a `**Supersedes:**` or `**Superseded-by:**` value, as stems
 *  (no directory, no `.md`). Markdown links and backticks are read through. Only
 *  words that are also in `known` count, so prose in the line cannot invent one. */
export function targetsOf(value, known) {
    if (value === null) return [];
    const flat = value.replace(/\[[^\]]*\]\(([^)]*)\)/g, " $1 ");
    const found = [];
    for (const word of flat.split(/[ \t,;()`*]+/)) {
        const stem = word.slice(word.lastIndexOf("/") + 1).replace(/[.]md[.]?$/, "").replace(/[.:]+$/, "");
        if (stem !== "" && known.has(stem) && !found.includes(stem)) found.push(stem);
    }
    return found;
}

/** The date in a spec's file name (`..._2026_10_21`), as `2026-10-21`, or `""`. */
export function dateOf(base) {
    const all = [...base.matchAll(/(\d{4})_(\d{2})_(\d{2})(?!\d)/g)];
    const last = all[all.length - 1];
    return last ? `${last[1]}-${last[2]}-${last[3]}` : "";
}

const stemOf = (base) => (base.endsWith(".md") ? base.slice(0, -3) : base);

/** What a spec replaces and what replaces it, as one table cell. Either spec may
 *  say it (`Supersedes` on the new one, `Superseded-by` on the old one). A value
 *  that cites a section (`§4.2 of X`) is a partial replacement, and says "part of". */
function relationsCell(stem, replaces, replacedBy, baseOf) {
    const list = (map, whole, partial) => {
        const found = map.get(stem);
        if (!found?.size) return [];
        const link = (t) => `[\`${t}\`](${baseOf.get(t)})`;
        const names = (isPartial) => [...found].filter(([, p]) => p === isPartial).map(([t]) => t).sort(byteCompare).map(link).join(", ");
        const out = [];
        if ([...found.values()].includes(false)) out.push(`${whole} ${names(false)}`);
        if ([...found.values()].includes(true)) out.push(`${partial} ${names(true)}`);
        return out;
    };
    return [...list(replaces, "replaces", "replaces part of"), ...list(replacedBy, "replaced by", "partly replaced by")].join("; ");
}

/** One table row for a spec file (basename as a latin1 byte string). */
export function rowFor(base, title, { status = NONE, date = "", relations = "" } = {}) {
    const word = status === NONE ? "none" : status;
    return `| [\`${stemOf(base)}\`](${base}) | ${title} | ${word} | ${date} | ${relations} |\n`;
}

/**
 * Render the generated section from rows grouped by area (`UNSORTED` is the
 * group of specs that name none, always last). `total` is the number of
 * candidate files. Returns `{ text, shown }`; the caller compares `shown` with
 * `total` (the completeness assertion).
 */
export function render(rowsByArea, total) {
    let shown = 0;
    let out = "";
    const emit = (s) => {
        out += bytes(s);
    };
    const emitRows = (rows) => {
        emit("| Spec | Title | Status | Date | Replaces / replaced by |\n|---|---|---|---|---|\n");
        for (const r of rows) out += r;
        shown += rows.length;
    };

    emit(`${BEGIN}\n`);
    emit("\n## All specs by area\n\n");
    emit("Generated by `scripts/gen-docs-index.sh` — do not hand-edit. Covers\n");
    emit("every spec directly in `docs/specs/`, which the curated sections\n");
    emit("above deliberately do not. Each spec is listed once, under the area its\n");
    emit("`**Area:**` line names, newest first. The Status column is its\n");
    emit("`**Status:**` word; read it before trusting the spec. A spec's\n");
    emit("`**Supersedes:**` and `**Superseded-by:**` lines show here as links in both\n");
    emit("directions (`docs/specs/README.md`).\n");
    emit('\n`archive/` is excluded on purpose — it means "not worth reading unless you\n');
    emit('are doing history". Everything else is here; the completeness\n');
    emit("assertion in the generator fails the build rather than emit a\n");
    emit("partial list.\n");

    const areas = [...rowsByArea.keys()].filter((k) => k !== UNSORTED && rowsByArea.get(k).length > 0).sort(byteCompare);
    for (const area of areas) {
        emit(`\n### ${area}\n\n`);
        emitRows(rowsByArea.get(area));
    }

    // Specs that name no area — surfaced, not dropped. Hiding them would make
    // the index look complete while omitting most of the tree.
    const unsorted = rowsByArea.get(UNSORTED);
    if (unsorted && unsorted.length > 0) {
        emit("\n### unsorted\n\n");
        emit("No `**Area:**` line yet (or one that is not a short plain name), newest\n");
        emit("first. Not a backlog to bulk-stamp — an area guessed from a title is worse\n");
        emit("than none. Add the line when you touch one and know which area it belongs to.\n\n");
        emitRows(unsorted);
    }

    emit(`\n${END}\n`);
    return { text: out, shown };
}

// ── Which files ARE the spec tree ───────────────────────────────────────────
//
// Tracked files, not a filesystem glob. The index describes the COMMITTED
// spec tree, and a bare `docs/specs/*.md` glob does not: it also picks up
// work-in-progress specs that are not in the repo yet. Regenerating with an
// untracked spec present would write a row for it into INDEX.md, and
// committing that INDEX.md without the spec leaves a link to a file nobody
// else has. It also made `--check` report STALE for a perfectly clean
// checkout. (#3073)
//
// `git ls-files` reads the INDEX, not HEAD, so a `git add`ed spec counts
// immediately — "write the spec, add it, regenerate, commit both" works.
//
// `-z` so paths arrive raw. Without it git quotes any path containing a
// non-ASCII byte ("docs/specs/Caf\303\251.md"), the depth-1 filter rejected
// the quoted form, and that spec silently had no row — which the completeness
// assertion could not see, since both of its totals came from the same list.
// (Spec §2.3; reproduced with the shell version.)
//
// Depth-1 only. A git pathspec's `*` matches `/` as well, so `docs/specs/*.md`
// would also pull in `docs/specs/archive/*.md` — the one subtree this index
// excludes on purpose. The filter, not the pathspec, bounds the depth.
//
// Deduplicated keeping first occurrence (Codex P2, PR #3073): during an
// unresolved merge conflict `git ls-files` lists a conflicted path once PER
// INDEX STAGE, which would render three identical rows — and the completeness
// assertion could not catch it, since both totals come from the same tripled
// list. Regenerating DURING a conflict is the documented way to resolve one.
//
// Tracked but deleted in the working tree (`rm` without `git rm`): no file to
// read, so no row built from a failed read.
//
// An EMPTY tracked list means this tree is not the one git found (Codex P2,
// PR #3073): unpack a source tarball inside an unrelated checkout and
// `git rev-parse` resolves the ancestor repo, which tracks none of these
// files. Taking the git answer would silently empty the generated section —
// and 0 == 0 satisfies the completeness assertion. The directory listing
// covers that and the plain not-a-repo case with one condition.
//
// Returns `[{ path: Buffer (raw bytes, relative to root), base: latin1 string }]`.
const DEPTH1_SPEC = /^docs\/specs\/[^/]+\.md$/;

function isRegularFile(root, rel) {
    try {
        return statSync(absPath(root, rel)).isFile();
    } catch {
        return false;
    }
}

/** Absolute path as a Buffer, so non-UTF-8 names survive on POSIX. */
function absPath(root, rel) {
    return Buffer.concat([Buffer.from(root.endsWith("/") || root.endsWith("\\") ? root : root + "/", "utf8"), rel]);
}

function git(root, args, env) {
    try {
        return execFileSync("git", args, {
            cwd: root,
            env,
            stdio: ["ignore", "pipe", "ignore"],
            maxBuffer: 256 * 1024 * 1024,
        });
    } catch {
        return null;
    }
}

function splitNul(buf) {
    const out = [];
    let start = 0;
    for (let i = 0; i < buf.length; i++) {
        if (buf[i] === 0) {
            if (i > start) out.push(buf.subarray(start, i));
            start = i + 1;
        }
    }
    if (start < buf.length) out.push(buf.subarray(start));
    return out;
}

function baseOf(latin1Path) {
    return latin1Path.slice(latin1Path.lastIndexOf("/") + 1);
}

export function specFiles(root, env = process.env) {
    if (git(root, ["rev-parse", "--git-dir"], env) !== null) {
        const listed = git(root, ["ls-files", "-z", "--", "docs/specs"], env);
        if (listed !== null) {
            const seen = new Set();
            const tracked = [];
            for (const p of splitNul(listed)) {
                const s = p.toString("latin1");
                if (!DEPTH1_SPEC.test(s) || seen.has(s)) continue;
                seen.add(s);
                tracked.push({ path: p, base: baseOf(s) });
            }
            if (tracked.length > 0) {
                return tracked.filter((f) => isRegularFile(root, f.path));
            }
        }
    }
    // Directory listing: what `docs/specs/*.md` expanded to under LC_ALL=C —
    // byte-sorted, no dotfiles, regular files only.
    let names;
    try {
        names = readdirSync(absPath(root, Buffer.from("docs/specs")), { encoding: "buffer" });
    } catch {
        return [];
    }
    return names
        .map((n) => ({ n, s: n.toString("latin1") }))
        .filter(({ s }) => s.endsWith(".md") && !s.startsWith("."))
        .sort((a, b) => Buffer.compare(a.n, b.n))
        .map(({ n, s }) => ({ path: Buffer.concat([Buffer.from("docs/specs/"), n]), base: s }))
        .filter((f) => isRegularFile(root, f.path));
}

/** Read up to the first HEAD_LINES lines of a file as a latin1 string.
 *  Unreadable → "" (the file still gets a row: `__none__`, filename title). */
function readHead(root, rel) {
    let fd;
    try {
        fd = openSync(absPath(root, rel), "r");
        const chunks = [];
        const buf = Buffer.alloc(64 * 1024);
        let newlines = 0;
        for (;;) {
            const n = readSync(fd, buf, 0, buf.length, null);
            if (n <= 0) break;
            const chunk = Buffer.from(buf.subarray(0, n));
            chunks.push(chunk);
            for (let i = 0; i < n; i++) if (chunk[i] === 0x0a) newlines++;
            if (newlines >= HEAD_LINES) break;
        }
        return Buffer.concat(chunks).toString("latin1");
    } catch {
        return "";
    } finally {
        if (fd !== undefined) closeSync(fd);
    }
}

/** Build the generated section for the tree at `root`. */
export function build(root, { env = process.env, transformRows } = {}) {
    const specs = [];
    for (const f of specFiles(root, env)) {
        if (f.base === "INDEX.md" || f.base === "README.md") continue;
        specs.push({ base: f.base, text: readHead(root, f.path) });
    }
    const baseOf = new Map(specs.map((x) => [stemOf(x.base), x.base]));
    const known = new Set(baseOf.keys());
    // Either side of a replacement is enough, so build both directions.
    const replaces = new Map();
    const replacedBy = new Map();
    const note = (map, key, value, partial) => {
        if (!map.has(key)) map.set(key, new Map());
        // Said whole anywhere, it is whole.
        map.get(key).set(value, (map.get(key).get(value) ?? true) && partial);
    };
    // A `§` (U+00A7, read as the byte pair C2 A7) means a section, not the spec.
    const cites = (value) => value !== null && value.includes("\xa7");
    for (const x of specs) {
        const stem = stemOf(x.base);
        const said = fieldOf(x.text, "supersedes");
        for (const old of targetsOf(said, known)) {
            if (old === stem) continue;
            note(replaces, stem, old, cites(said));
            note(replacedBy, old, stem, cites(said));
        }
        const by = fieldOf(x.text, "superseded-by");
        for (const next of targetsOf(by, known)) {
            if (next === stem) continue;
            note(replacedBy, stem, next, cites(by));
            note(replaces, next, stem, cites(by));
        }
    }
    const entries = specs.map((x) => {
        const stem = stemOf(x.base);
        const date = dateOf(x.base);
        const row = rowFor(x.base, titleOf(x.text, x.base), {
            status: statusOf(x.text),
            date,
            relations: relationsCell(stem, replaces, replacedBy, baseOf),
        });
        return { area: areaOf(x.text), base: x.base, date, row };
    });
    // Newest first inside a group; undated and equal dates by name.
    entries.sort((a, b) => byteCompare(b.date, a.date) || byteCompare(a.base, b.base));
    const rowsByArea = new Map();
    for (const e of entries) {
        if (!rowsByArea.has(e.area)) rowsByArea.set(e.area, []);
        rowsByArea.get(e.area).push(e.row);
    }
    // Test seam for the completeness assertion, which cannot trip on a correct
    // tree by construction. Never set outside tests.
    if (transformRows) transformRows(rowsByArea);
    const { text, shown } = render(rowsByArea, specs.length);
    return { text, total: specs.length, shown };
}

// ── Why --check is scoped to branches that touch docs/specs ─────────────────
//
// INDEX.md is generated but committed, so it is a shared mutable file that
// every spec-touching PR writes. An unscoped assertion made ANY staleness on
// main fail the gate on EVERY open PR, including PRs that touch no docs at
// all — the PR that caused it is never the PR that fails (issue #3059: five
// regeneration PRs in one session, each blocking unrelated work).
//
// Scoping it makes the failure attributable: a branch that adds a spec or
// changes a Status is the one that must regenerate, and it is the only one
// asked to. A branch that touches no specs cannot make the index stale, so it
// is not held responsible for someone else's omission. This matches the
// sibling doc gates (`check-doc-status.sh`, `check-spec-citations.sh`), which
// are changed-files-only.
//
// #3056 removed the section-header counts, the only part of the file whose
// correctness depended on what OTHER branches did. Rows are per-line and merge
// correctly, so concurrent regenerations compose. What that cannot cover, and
// this does: a branch that adds a spec and never regenerates at all.
//
// --name-status, and BOTH sides of a rename (Codex P2 on PR #3069):
// --name-only reports only a rename's DESTINATION, so moving a spec out of
// docs/specs/ showed up as a non-spec path, skipped the check, and left the
// index carrying a row for a file no longer there. Deliberately UNLIKE
// check-doc-status.sh, which skips pure renames: here a rename is exactly the
// kind of change that alters the index, since the row is keyed on the filename.
//
// The generator itself is in scope too (both files): a change to it can alter
// the output with no spec touched. INDEX.md itself counts as well, so a
// hand-edit to it is still caught. So does `.gitattributes`: it decides the
// bytes a checkout hands the generator (e.g. `eol=crlf` on docs/specs), so an
// attributes-only change can make the committed index unreproducible (Codex
// P2, #3590).
const IN_SCOPE = /^(docs\/specs\/.*[.]md|scripts\/gen-docs-index\.(sh|mjs)|\.gitattributes)$/;

/** Returns `{ check: boolean, message: string[] }`. */
export function shouldCheck(root, env) {
    const base = env.GITHUB_BASE_REF || "main";
    let ref;
    if (git(root, ["rev-parse", "--verify", "--quiet", `origin/${base}`], env) !== null) {
        ref = `origin/${base}`;
    } else if (git(root, ["rev-parse", "--verify", "--quiet", base], env) !== null) {
        ref = base;
    } else {
        // Same posture as check-doc-status.sh: an unresolvable base is a CI
        // environment problem, not a docs problem. Skip rather than fail.
        return { check: false, message: [`gen-docs-index: cannot resolve base ref '${base}' — skipping.`] };
    }
    const diff = git(root, ["diff", "--name-status", "-z", "--find-renames", `${ref}...HEAD`], env);
    if (diff !== null) {
        const fields = splitNul(diff).map((b) => b.toString("latin1"));
        for (let i = 0; i < fields.length;) {
            const status = fields[i++];
            const nPaths = status.startsWith("R") || status.startsWith("C") ? 2 : 1;
            for (let k = 0; k < nPaths && i < fields.length; k++) {
                if (IN_SCOPE.test(fields[i++])) return { check: true, message: [] };
            }
        }
    }
    return {
        check: false,
        message: [
            "gen-docs-index: no specs changed on this branch — index not asserted.",
            "  (run 'bash scripts/gen-docs-index.sh --check-all' to assert anyway)",
        ],
    };
}

/** Keep everything before the first line containing the BEGIN marker; with no
 *  marker, keep the whole file and append a horizontal rule. */
export function spliceHead(indexText) {
    const at = indexText.indexOf(bytes(BEGIN));
    if (at === -1) return indexText + "\n---\n\n";
    return indexText.slice(0, indexText.lastIndexOf("\n", at) + 1);
}

// ── STALE excerpt ───────────────────────────────────────────────────────────
//
// Show WHAT differs, not just that something does. A gate that only says
// "stale" makes the reader re-derive the diff by hand — and when it fails on
// CI but not locally, that guesswork is the whole cost of the failure.
// Produced here rather than by diff(1), which is not guaranteed on Windows
// outside Git Bash. Format follows diff's normal output (`3c3`, `<`, `---`, `>`).

/** Normal-format line diff of two line arrays. Common prefix/suffix are
 *  trimmed first; the middle is LCS-aligned when small enough, otherwise
 *  reported as one change hunk (still correct, just coarser). */
export function lineDiff(a, b) {
    let pre = 0;
    while (pre < a.length && pre < b.length && a[pre] === b[pre]) pre++;
    let suf = 0;
    while (suf < a.length - pre && suf < b.length - pre && a[a.length - 1 - suf] === b[b.length - 1 - suf]) suf++;
    const A = a.slice(pre, a.length - suf);
    const B = b.slice(pre, b.length - suf);

    // Edit script as runs of [kind, aStart, aEnd, bStart, bEnd] (0-based, end-exclusive, within A/B).
    const ops = [];
    if (A.length * B.length <= 4_000_000) {
        const n = A.length;
        const m = B.length;
        const w = m + 1;
        const lcs = new Uint32Array((n + 1) * (m + 1));
        for (let i = n - 1; i >= 0; i--) {
            for (let j = m - 1; j >= 0; j--) {
                lcs[i * w + j] =
                    A[i] === B[j] ? lcs[(i + 1) * w + j + 1] + 1 : Math.max(lcs[(i + 1) * w + j], lcs[i * w + j + 1]);
            }
        }
        let i = 0;
        let j = 0;
        let ai = 0;
        let bj = 0;
        const flush = () => {
            if (i > ai || j > bj) ops.push([ai, i, bj, j]);
        };
        while (i < n || j < m) {
            if (i < n && j < m && A[i] === B[j]) {
                flush();
                i++;
                j++;
                ai = i;
                bj = j;
            } else if (j < m && (i === n || lcs[i * w + j + 1] >= lcs[(i + 1) * w + j])) {
                j++;
            } else {
                i++;
            }
        }
        flush();
    } else if (A.length > 0 || B.length > 0) {
        ops.push([0, A.length, 0, B.length]);
    }

    const range = (s, e) => (e - s === 1 ? `${s + 1}` : `${s + 1},${e}`);
    const out = [];
    for (const [as, ae, bs, be] of ops) {
        const a0 = pre + as;
        const a1 = pre + ae;
        const b0 = pre + bs;
        const b1 = pre + be;
        if (a1 === a0) {
            out.push(`${a0}a${range(b0, b1)}`);
        } else if (b1 === b0) {
            out.push(`${range(a0, a1)}d${b0}`);
        } else {
            out.push(`${range(a0, a1)}c${range(b0, b1)}`);
        }
        for (let k = a0; k < a1; k++) out.push(`< ${a[k]}`);
        if (a1 > a0 && b1 > b0) out.push("---");
        for (let k = b0; k < b1; k++) out.push(`> ${b[k]}`);
    }
    return out;
}

function splitLines(text) {
    const lines = text.split("\n");
    if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
    return lines;
}

// ── Atomic write ────────────────────────────────────────────────────────────

function sleepMs(ms) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

/** Write via a temp file in the same directory, then rename over the target.
 *  Rename is atomic on every OS; on Windows it can fail transiently while
 *  another process (editor, indexer, antivirus) has the file open, so retry
 *  briefly before giving up. */
function writeAtomic(target, buf) {
    const tmp = `${target}.tmp-${process.pid}-${Date.now().toString(36)}`;
    writeFileSync(tmp, buf);
    let lastErr;
    for (let attempt = 0; attempt < 20; attempt++) {
        try {
            renameSync(tmp, target);
            return;
        } catch (e) {
            lastErr = e;
            if (!["EPERM", "EACCES", "EBUSY"].includes(e.code)) break;
            sleepMs(50);
        }
    }
    rmSync(tmp, { force: true });
    throw lastErr;
}

// ── Main ────────────────────────────────────────────────────────────────────

/**
 * Run the generator. Returns the exit code.
 * `io.out` / `io.err` receive Buffers or strings; defaults are the process
 * streams. `root` defaults to the current directory, as the shell version did.
 */
export function main(argv, { root = process.cwd(), env = process.env, out, err, transformRows } = {}) {
    const write = (stream, fallback) => (s) => (stream ? stream(s) : fallback.write(s));
    const stdout = write(out, process.stdout);
    const stderr = write(err, process.stderr);
    const line = (w) => (s) => w(`${s}\n`);
    const say = line(stdout);
    const warn = line(stderr);

    // Only the first argument is read, as before; anything else regenerates.
    const mode = argv[0] === "--check" ? "check" : argv[0] === "--check-all" ? "check-all" : "write";

    // Nothing to assert on this branch — exit before doing the work.
    if (mode === "check") {
        const { check, message } = shouldCheck(root, env);
        if (!check) {
            message.forEach(say);
            return 0;
        }
    }

    const indexPath = join(root, INDEX);
    if (!existsSync(indexPath) || !statSync(indexPath).isFile()) {
        say(`gen-docs-index: ${INDEX} not found`);
        return 1;
    }

    const current = readFileSync(indexPath);
    const { text, total, shown } = build(root, { env, transformRows });

    // Completeness assertion — the check that would have caught the 189
    // vanished specs, and the reason they cannot come back. Every candidate
    // file must be accounted for by exactly one emitted row; if not, refuse to
    // produce the index at all. Always report the count: a count that differs
    // between two machines is the first thing worth knowing when --check
    // disagrees with a local run.
    warn(`gen-docs-index: ${total} specs under docs/specs/ (excluding archive/)`);
    if (shown !== total) {
        warn(`gen-docs-index: FATAL — emitted ${shown} rows for ${total} specs;`);
        warn(`  ${total - shown} unaccounted for. The index would be incomplete`);
        warn("  while claiming to cover every file; refusing to write it.");
        // INDEX.md is left exactly as it was: a stale index is recoverable,
        // a confidently-incomplete one is not.
        return 3;
    }

    const next = toBuffer(spliceHead(current.toString("latin1")) + text);

    if (mode !== "write") {
        if (current.equals(next)) {
            say("gen-docs-index: INDEX.md is current.");
            return 0;
        }
        say("gen-docs-index: INDEX.md is STALE.");
        say("  A spec was added, removed, or had its Status changed without");
        say("  regenerating the index. Run: bash scripts/gen-docs-index.sh");
        say("");
        say("  Difference (committed < vs regenerated >), first 40 lines:");
        const diff = lineDiff(splitLines(current.toString("latin1")), splitLines(next.toString("latin1")));
        for (const d of diff.slice(0, 40)) stdout(Buffer.concat([toBuffer(`    ${d}`), Buffer.from("\n")]));
        return 1;
    }

    writeAtomic(indexPath, next);
    say(`gen-docs-index: regenerated ${INDEX}`);
    return 0;
}

if (isMain(import.meta.url)) {
    process.exitCode = main(process.argv.slice(2));
}
