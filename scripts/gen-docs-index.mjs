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
// WHY GENERATED, AND WHY GROUPED BY STATUS
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
    realpathSync,
    renameSync,
    rmSync,
    statSync,
    writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const INDEX = "docs/specs/INDEX.md";
export const BEGIN = "<!-- BEGIN GENERATED INDEX — edit scripts/gen-docs-index.sh, not this section -->";
export const END = "<!-- END GENERATED INDEX -->";

/** Canonical buckets, in output order. */
export const CANONICAL = ["implemented", "active", "proposed", "draft", "living", "historical", "superseded"];
const NONE = "__none__";

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

/** One table row for a spec file (basename as a latin1 byte string). */
export function rowFor(base, title) {
    const stem = base.endsWith(".md") ? base.slice(0, -3) : base;
    return `| [\`${stem}\`](${base}) | ${title} |\n`;
}

/**
 * Render the generated section from rows grouped by Status. `total` is the
 * number of candidate files. Returns `{ text, shown }`; the caller compares
 * `shown` with `total` (the completeness assertion).
 */
export function render(rowsByStatus, total) {
    let shown = 0;
    let out = "";
    const emit = (s) => {
        out += bytes(s);
    };
    const emitRows = (rows) => {
        for (const r of rows) out += r;
        shown += rows.length;
    };

    emit(`${BEGIN}\n`);
    emit("\n## All specs by status\n\n");
    emit("Generated by `scripts/gen-docs-index.sh` — do not hand-edit. Covers\n");
    emit("every spec directly in `docs/specs/`, which the curated sections\n");
    emit("above deliberately do not.\n");
    emit('\n`archive/` is excluded on purpose — it means "not worth reading unless you\n');
    emit('are doing history". Everything else is here; the completeness\n');
    emit("assertion in the generator fails the build rather than emit a\n");
    emit("partial list.\n");

    for (const st of CANONICAL) {
        const rows = rowsByStatus.get(st);
        if (!rows || rows.length === 0) continue;
        emit(`\n### ${st}\n\n`);
        emit("| Spec | Title |\n|---|---|\n");
        emitRows(rows);
    }

    // Specs with no Status line at all — surfaced, not dropped. Hiding them
    // would make the index look complete while omitting a sixth of the tree.
    const none = rowsByStatus.get(NONE);
    if (none && none.length > 0) {
        emit("\n### no status line\n\n");
        emit("Predate the closed vocabulary. Not a backlog to bulk-restamp —\n");
        emit('an unverified restamp turns "unknown" into "confidently wrong".\n');
        emit("Fix one when you touch it and know its real state.\n\n");
        emit("| Spec | Title |\n|---|---|\n");
        emitRows(none);
    }

    // Status line present, but its first word is outside the closed enum.
    //
    // An earlier version printed ONLY the seven canonical buckets, so every one
    // of these landed in a bucket the print loop never iterated and vanished
    // from the index with no trace — 189 specs (26% of the tree) silently
    // absent, while the header above claimed to cover every file. --check could
    // not catch it: it only diffs a re-run of the same logic, so a stable bug
    // stays green forever. The completeness assertion is what catches it.
    const noncanon = [...rowsByStatus.keys()]
        .filter((st) => st !== NONE && !CANONICAL.includes(st) && rowsByStatus.get(st).length > 0)
        .sort(byteCompare);
    if (noncanon.length > 0) {
        emit("\n### non-canonical status\n");
        emit("\nThese carry a `**Status:**` line whose first word is not in the closed enum\n");
        emit("(`docs/specs/README.md`). Grouped by the word actually found, so the\n");
        emit("real state is visible rather than guessed at. `check-doc-status.sh`\n");
        emit("requires a fix the next time one of these is edited; as with the\n");
        emit("section above, do not bulk-restamp them.\n");
        for (const st of noncanon) {
            // `st` is already a byte string of a–z only.
            out += `\n**\`${st}\`**\n\n`;
            emit("| Spec | Title |\n|---|---|\n");
            emitRows(rowsByStatus.get(st));
        }
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
    const rowsByStatus = new Map();
    let total = 0;
    for (const f of specFiles(root, env)) {
        if (f.base === "INDEX.md" || f.base === "README.md") continue;
        total++;
        const text = readHead(root, f.path);
        const st = statusOf(text);
        const row = rowFor(f.base, titleOf(text, f.base));
        if (!rowsByStatus.has(st)) rowsByStatus.set(st, []);
        rowsByStatus.get(st).push(row);
    }
    // Test seam for the completeness assertion, which cannot trip on a correct
    // tree by construction. Never set outside tests.
    if (transformRows) transformRows(rowsByStatus);
    const { text, shown } = render(rowsByStatus, total);
    return { text, total, shown };
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

function isEntryPoint() {
    if (!process.argv[1]) return false;
    try {
        const self = realpathSync(fileURLToPath(import.meta.url));
        const invoked = realpathSync(resolve(process.argv[1]));
        return process.platform === "win32" ? self.toLowerCase() === invoked.toLowerCase() : self === invoked;
    } catch {
        return false;
    }
}

if (isEntryPoint()) {
    process.exitCode = main(process.argv.slice(2));
}
