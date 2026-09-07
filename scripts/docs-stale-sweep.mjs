#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// docs-stale-sweep.mjs — the recurring sweep of SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md
// Phase 5 (item 2): flag docs that (a) claim to be current (Status is not
// `historical`/`superseded`), (b) have not been touched in more than N weeks,
// and (c) cite a specific file that HAS changed since — or that no longer
// exists at all. Surfaces candidates for a human or agent to triage; never
// edits anything. A cheap git-recency check on the cited paths, not a re-read.
//
// Why this exists: two manual audits (2026-08-07, 2026-09-06) each found the
// same class of rot — Status lines and file citations that were true when
// written and silently stopped being true. `check-doc-status.sh` stops the
// backlog GROWING (it gates changed files on PRs); nothing re-verified a doc
// once it was merged. This is that re-verification, on a schedule
// (`.github/workflows/docs-stale-sweep.yml`), with the output posted to one
// standing issue rather than a third one-off report nobody acts on.
//
// Usage:
//   node scripts/docs-stale-sweep.mjs [--weeks N] [--limit K] [--json] [--root DIR]
//
//   --weeks N   untouched-for threshold (default 8)
//   --limit K   rows in the markdown table (default 60; the count is always full)
//   --json      full structured output instead of markdown
//   --root DIR  repo root (default: cwd)
//
// Exit code is 0 whenever the sweep ran — it is a triage list, not a gate.
// The one-pass `git log --name-only` it relies on needs full history
// (`fetch-depth: 0` in CI); a shallow clone makes every path look untouched
// since the clone and the sweep says so instead of guessing.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const TERMINAL_STATUSES = new Set(["historical", "superseded"]);
const CANONICAL_STATUSES = new Set(["draft", "proposed", "active", "implemented", "living", "historical", "superseded"]);
const CITATION_EXTS = "rs|ts|tsx|mjs|cjs|js|sh|json|yml|yaml|toml|md";

/**
 * Pull the Status word out of a doc's text the same way gen-docs-index.sh
 * does: first `**Status:**` line, first word, lowercased, letters only.
 * Returns null when there is no Status line at all.
 */
export function statusOf(text) {
    const m = text.match(/^\*\*Status:\*\*[ \t]*(\S*)/im);
    if (!m) return null;
    return m[1].toLowerCase().replace(/[^a-z]/g, "");
}

/**
 * Every file-looking token a doc cites. Two shapes count:
 *  - anything in backticks that ends in a source/config extension
 *    (`agentmux-srv/src/server/reactive.rs`, `runner.rs`)
 *  - a bare slash-containing path ending in one (agentmux-srv/src/x.rs), for
 *    docs that don't backtick their citations.
 * A trailing `:123` / `:120-140` line reference is stripped. Returned
 * deduplicated, in first-seen order.
 */
export function extractCitations(text) {
    const out = new Set();
    const ticked = new RegExp("`([A-Za-z0-9_./-]+\\.(?:" + CITATION_EXTS + "))(?::[0-9][0-9-]*)?`", "g");
    for (const m of text.matchAll(ticked)) out.add(normalize(m[1]));
    const bare = new RegExp("(?:^|[\\s(])((?:[A-Za-z0-9_.-]+/)+[A-Za-z0-9_.-]+\\.(?:" + CITATION_EXTS + "))(?=[\\s):,;]|$)", "gm");
    for (const m of text.matchAll(bare)) out.add(normalize(m[1]));
    return [...out].filter(plausibleCitation);
}

function normalize(token) {
    let t = token.replace(/^\.\//, "");
    while (t.startsWith("../")) t = t.slice(3);
    return t;
}

/**
 * Is this token something that could name a repo file at all? Filters the
 * shapes a first run turned up as noise: absolute paths (`/path/to/foo.ts`,
 * `/workspace/foo.ts` — examples, not citations), extension lists written
 * with a slash (`.ts/.tsx`), and anything whose basename is only an
 * extension.
 */
export function plausibleCitation(token) {
    if (token.startsWith("/")) return false;
    const base = token.slice(token.lastIndexOf("/") + 1);
    return /^[A-Za-z0-9_-][A-Za-z0-9_.-]*\.[A-Za-z0-9]+$/.test(base);
}

/**
 * Map a cited token onto a tracked repo path.
 *   exact tracked path            -> { kind: "resolved", path }
 *   unique tracked path ending in "/<token>" (token has a slash) -> resolved
 *   unique tracked basename (token is a bare filename)           -> resolved
 *   several candidates            -> { kind: "ambiguous" }
 *   none                          -> { kind: "absent" }
 * Ambiguous and absent are kept apart on purpose: `reducer/tests.rs` matching
 * three files is not evidence the file is gone, and "missing" is the report's
 * strongest signal (codex P2 on #3068). `tracked` is a Set of repo-relative
 * paths; `byBasename` maps basename -> array of paths.
 */
export function resolveCitation(token, tracked, byBasename) {
    if (tracked.has(token)) return { kind: "resolved", path: token };
    let hits;
    if (token.includes("/")) {
        const suffix = "/" + token;
        hits = [];
        for (const p of tracked) if (p.endsWith(suffix)) hits.push(p);
    } else {
        hits = byBasename.get(token) ?? [];
    }
    if (hits.length === 1) return { kind: "resolved", path: hits[0] };
    return { kind: hits.length === 0 ? "absent" : "ambiguous" };
}

/**
 * Classify one doc. `touched` maps path -> unix seconds of its last commit.
 * Returns null for docs the sweep does not judge (terminal Status, or no
 * Status at all — those are `gen-docs-index.sh`'s backlog sections, counted
 * separately by the caller).
 */
export function classifyDoc({ doc, text, touched, tracked, byBasename, now, weeks }) {
    const status = statusOf(text);
    if (status === null || TERMINAL_STATUSES.has(status)) return null;
    const docTouched = touched.get(doc);
    const ageDays = docTouched === undefined ? null : Math.floor((now - docTouched) / 86400);
    const stale = ageDays !== null && ageDays > weeks * 7;
    const drifted = [];
    const missing = [];
    // Several tokens can name one file (`runner.rs`, `migrations/runner.rs`,
    // the full path); a file counts once however many ways it is cited.
    const seen = new Set();
    for (const token of extractCitations(text)) {
        if (token === doc) continue;
        const r = resolveCitation(token, tracked, byBasename);
        if (r.kind === "ambiguous") continue;
        if (r.kind === "absent") {
            // Only a path with a directory in it is a confident "this file
            // is gone" — a bare `config.rs` that matches nothing is not
            // evidence of anything.
            if (token.includes("/")) missing.push(token);
            continue;
        }
        const resolved = r.path;
        if (resolved === doc || seen.has(resolved)) continue;
        seen.add(resolved);
        const t = touched.get(resolved);
        if (t !== undefined && docTouched !== undefined && t > docTouched) {
            drifted.push({ path: resolved, daysAfterDoc: Math.floor((t - docTouched) / 86400) });
        }
    }
    drifted.sort((a, b) => b.daysAfterDoc - a.daysAfterDoc);
    return {
        doc,
        status,
        canonical: CANONICAL_STATUSES.has(status),
        ageDays,
        stale,
        drifted,
        missing,
        flagged: stale && (drifted.length > 0 || missing.length > 0),
    };
}

function git(root, args) {
    return execFileSync("git", args, { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
}

/** path -> unix seconds of the most recent commit touching it, in ONE pass. */
export function lastTouchedMap(root) {
    const out = new Map();
    let ct = 0;
    for (const line of git(root, ["log", "--format=@%ct", "--name-only"]).split("\n")) {
        if (line.startsWith("@")) ct = Number(line.slice(1));
        else if (line && !out.has(line)) out.set(line, ct);
    }
    return out;
}

/**
 * Ask git, rather than infer: a shallow clone's `git log --name-only` still
 * lists every path at the boundary commit, so the map is never empty there
 * (codex P2 on #3068).
 */
function isShallow(root) {
    return git(root, ["rev-parse", "--is-shallow-repository"]).trim() === "true";
}

export function sweep({ root = process.cwd(), weeks = 8, now = Math.floor(Date.now() / 1000) } = {}) {
    const shallow = isShallow(root);
    const trackedList = git(root, ["ls-files", "-z"]).split("\0").filter(Boolean);
    const tracked = new Set(trackedList);
    const byBasename = new Map();
    for (const p of trackedList) {
        const b = path.posix.basename(p);
        if (!byBasename.has(b)) byBasename.set(b, []);
        byBasename.get(b).push(p);
    }
    const touched = lastTouchedMap(root);
    const docs = trackedList.filter((p) => p.startsWith("docs/") && p.endsWith(".md") && !p.split("/").includes("archive"));

    const results = [];
    let noStatus = 0;
    let terminal = 0;
    for (const doc of docs) {
        const text = fs.readFileSync(path.join(root, doc), "utf8");
        const status = statusOf(text);
        if (status === null) {
            noStatus += 1;
            continue;
        }
        if (TERMINAL_STATUSES.has(status)) {
            terminal += 1;
            continue;
        }
        results.push(classifyDoc({ doc, text, touched, tracked, byBasename, now, weeks }));
    }
    const flagged = results.filter((r) => r.flagged);
    flagged.sort((a, b) => b.missing.length - a.missing.length || b.drifted.length - a.drifted.length || b.ageDays - a.ageDays);
    return {
        generatedAt: new Date(now * 1000).toISOString(),
        weeks,
        shallow,
        counts: {
            docs: docs.length,
            noStatus,
            terminal,
            live: results.length,
            stale: results.filter((r) => r.stale).length,
            flagged: flagged.length,
            nonCanonicalStatus: results.filter((r) => !r.canonical).length,
        },
        flagged,
    };
}

export function renderMarkdown(report, limit = 60) {
    const c = report.counts;
    const lines = [];
    lines.push(`# Docs stale-sweep — ${report.generatedAt.slice(0, 10)}`);
    lines.push("");
    lines.push(
        `Automated triage list (SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md Phase 5, ` +
            `\`scripts/docs-stale-sweep.mjs\`). A row means: the doc says it is current, ` +
            `nobody has touched it in more than **${report.weeks} weeks**, and at least one file it cites ` +
            `has changed since (or no longer exists). Each row is a *candidate* — open the doc, ` +
            `spot-verify, then either update it, restamp it \`historical\`/\`superseded\`, or leave it ` +
            `and expect it here again next week.`,
    );
    lines.push("");
    if (report.shallow) {
        lines.push("> **No git history was available** (shallow clone?). Every path reads as untouched; this run is not meaningful.");
        lines.push("");
    }
    lines.push(`| | |`);
    lines.push(`|---|---|`);
    lines.push(`| docs scanned (\`docs/**/*.md\`, excluding \`archive/\`) | ${c.docs} |`);
    lines.push(`| claiming to be current (Status not historical/superseded) | ${c.live} |`);
    lines.push(`| … of which untouched > ${report.weeks} weeks | ${c.stale} |`);
    lines.push(`| … **of which cite a file that changed since, or is gone** | **${c.flagged}** |`);
    lines.push(`| terminal (historical/superseded), not judged | ${c.terminal} |`);
    lines.push(`| no Status line at all (see \`docs/specs/INDEX.md\`) | ${c.noStatus} |`);
    lines.push(`| live but with a non-canonical Status word | ${c.nonCanonicalStatus} |`);
    lines.push("");
    if (report.flagged.length === 0) {
        lines.push("Nothing flagged.");
        return lines.join("\n") + "\n";
    }
    lines.push(`## Flagged (${report.flagged.length}, sorted: missing citations, then most drift, then oldest)`);
    lines.push("");
    lines.push("| Doc | Status | Untouched | Cited files changed since | Cited files missing |");
    lines.push("|---|---|---|---|---|");
    for (const r of report.flagged.slice(0, limit)) {
        const drift = r.drifted
            .slice(0, 4)
            .map((d) => `\`${d.path}\` (+${d.daysAfterDoc}d)`)
            .join("<br>") + (r.drifted.length > 4 ? `<br>… ${r.drifted.length - 4} more` : "");
        const miss = r.missing
            .slice(0, 4)
            .map((m) => `\`${m}\``)
            .join("<br>") + (r.missing.length > 4 ? `<br>… ${r.missing.length - 4} more` : "");
        lines.push(`| \`${r.doc}\` | ${r.status} | ${r.ageDays}d | ${drift || "—"} | ${miss || "—"} |`);
    }
    if (report.flagged.length > limit) {
        lines.push("");
        lines.push(`… and ${report.flagged.length - limit} more (run with \`--json\` or a larger \`--limit\`).`);
    }
    lines.push("");
    lines.push(
        "`(+Nd)` = the cited file's last commit is N days after the doc's last commit. " +
            "A missing citation is the strongest signal: the doc points at a path that is not in the repo.",
    );
    return lines.join("\n") + "\n";
}

function parseArgs(argv) {
    const opts = { weeks: 8, limit: 60, json: false, root: process.cwd() };
    for (let i = 0; i < argv.length; i += 1) {
        const a = argv[i];
        if (a === "--json") opts.json = true;
        else if (a === "--weeks") opts.weeks = Number(argv[++i]);
        else if (a === "--limit") opts.limit = Number(argv[++i]);
        else if (a === "--root") opts.root = argv[++i];
        else if (a === "--help" || a === "-h") {
            console.log("usage: docs-stale-sweep.mjs [--weeks N] [--limit K] [--json] [--root DIR]");
            process.exit(0);
        } else {
            console.error(`unknown argument: ${a}`);
            process.exit(2);
        }
    }
    if (!Number.isFinite(opts.weeks) || opts.weeks <= 0) {
        console.error("--weeks must be a positive number");
        process.exit(2);
    }
    return opts;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
    const opts = parseArgs(process.argv.slice(2));
    const report = sweep({ root: opts.root, weeks: opts.weeks });
    process.stdout.write(opts.json ? JSON.stringify(report, null, 2) + "\n" : renderMarkdown(report, opts.limit));
}
