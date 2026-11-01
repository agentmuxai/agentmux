#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-doc-references.mjs — CI check that what a doc says about the code is in the code.
//
// The architecture audit of 2026-11-01 (finding 6) found docs citing files that had
// moved, symbols that were renamed and line numbers that had drifted. A reader trusts a
// path in backticks; this checks it. It reads Markdown (`.md`, `.mdx`) under the
// directories given with `--docs` and checks the code in the tree given with `--code`
// (default: this repo), so the docs repo can run it against its pinned checkout of the code.
//
// WHAT IT CHECKS (each finding has a kind, shown in the report):
//
//   missing-path    a path in backticks that starts at a top-level folder of the code
//                   (`crates/`, `frontend/`, `docs/`, `scripts/`...) names no file or folder.
//                   `specs/...` is checked too: the specs live under `docs/specs/`.
//   line-citation   a file name with `:123` or `:12-40` after it, with `#L12`, or followed by
//                   ", line 437".
//                   A line number drifts with the next edit: cite the symbol instead.
//   missing-symbol  an identifier in backticks (`snake_case`, `CamelCase`, `Type::member`,
//                   `name()`, `UPPER_SNAKE`) that is not in the code. On a line that cites
//                   files, it must be in one of them; on a line that cites none, anywhere
//                   in the code (the Rust, TypeScript, config and script files, not docs)
//                   with `--any-symbol`, which a docs repo about this code turns on (the
//                   docs of the code itself also name Chromium's and CEF's identifiers).
//   table-missing-file / table-unknown-file
//                   under a heading that names a folder in backticks, the first table lists
//                   the files of that folder: every file in the folder that is not a test
//                   must be a row, and every row must be a file there.
//   fact            with `--facts`: a count, a range or a list of names a page states that
//                   the code gives (lib/doc-facts.mjs lists them: the migrations range, the
//                   host commands, the MuxAPI tools...).
//   denied-term     a term of the `--deny` file (anywhere, a code block too), which holds one `term => what to say
//                   instead` per line, for names the docs must not use any more.
//
// WHAT IT SKIPS. Front matter and fenced code blocks (a shell session is not a citation);
// a path with `<placeholders>`, `*`, `{a,b}` or `$VARS` (not a single file); a name that is
// plain lowercase words (a command, not a symbol); anything in the `--allow` file, which
// holds one term per line for names that are real but live outside the code (another
// product's setting, say). Prose claims are not checked.
//
// BASELINE. With `--baseline <file>` the findings of today are recorded and may only
// shrink (as check-file-sizes.mjs does for file length): a finding that is not in the
// baseline fails, in any doc, since a change to the code can break a doc it did not touch;
// a baseline entry that is no longer a finding fails only for a doc this change touches
// (main moves all day), and `--update` drops it. Without `--baseline` every finding fails.
//
// Usage:
//   node scripts/check-doc-references.mjs                       the docs of this repo
//   node scripts/check-doc-references.mjs --docs <dir> [--docs <dir>] [--code <root>]
//                                         [--deny <file>] [--allow <file>] [--baseline <file>] [--facts] [--any-symbol]
//   ... --update   lower the baseline (drop entries that are fixed; never adds)
//   ... --init     write the baseline from today's findings, adding entries

import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";
import { isSourcePath } from "./check-file-sizes.mjs";
import { checkFacts } from "./lib/doc-facts.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Folders of this repo whose Markdown is checked when `--docs` is not given. */
const DEFAULT_DOCS = ["README.md", "CONTRIBUTING.md", "BUILD.md", "docs"];

// A living doc is one a reader is expected to trust as current. Dated records (the analysis,
// retro, report and archive folders, and any name with a date in it) cite what existed when
// they were written, and rewriting them would falsify the record.
const DATED_FOLDER = /^docs\/(?:[a-z]+\/)?(?:analysis|retro|reports|archive|research|incident|investigations|status|plans|specs)\//;
const DATED_NAME = /[-_][0-9]{4}[-_][0-9]{2}([-_][0-9]{2})?/;
const EXCLUDED_FILE = /^(?:VERSION_HISTORY|CHANGELOG)\.md$|(?:^|\/)INDEX\.md$/;

/** Whether a repo-relative path is a doc that must stay true (see the note above). */
export function isLivingDoc(path) {
    return !DATED_FOLDER.test(path) && !DATED_NAME.test(path.split("/").pop()) && !EXCLUDED_FILE.test(path);
}

const CODE_EXT = /\.(?:rs|ts|tsx|js|mjs|cjs|jsx|json|toml|yml|yaml|sh|css|scss|sql|proto|html|ps1|cmd|iss|plist)$/;
const CITED_EXT = "rs|ts|tsx|js|mjs|cjs|jsx|json|toml|yml|yaml|sh|css|sql|md|mdx|html";
const LINE_SPAN = new RegExp(`^(.+\\.(?:${CITED_EXT}))(?::\\d+(?::\\d+)?(?:-\\d+)?|#L\\d+(?:-L?\\d+)?)$`);
const LINE_PROSE = new RegExp(`\`[^\`\\n]*\\.(?:${CITED_EXT})\`[^\`\\n]{0,30}?\\blines?\\s+\\d+`, "i");
const PLACEHOLDER = /[<>*{}$]|\.\.\./;
const TEXT_EXT = /\.(?:rs|ts|tsx|js|mjs|cjs|jsx|json|toml|yml|yaml|sh|css|scss|sql|proto|html)$/;

// ── Reading a doc ───────────────────────────────────────────────────────────

/**
 * The lines of a doc that are prose: `{ n, text }` (1-based), without front matter and
 * fenced code blocks.
 */
export function proseLines(text) {
    const lines = text.split("\n");
    const out = [];
    let i = 0;
    if (lines[0]?.trim() === "---") {
        i = 1;
        while (i < lines.length && lines[i].trim() !== "---") i++;
        i++;
    }
    let fence = null;
    for (; i < lines.length; i++) {
        const t = lines[i];
        const f = /^\s*(`{3,}|~{3,})/.exec(t);
        if (fence) {
            if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
            continue;
        }
        if (f) {
            fence = f[1];
            continue;
        }
        out.push({ n: i + 1, text: t });
    }
    return out;
}

/** The inline code spans of a line. */
export function codeSpans(line) {
    const out = [];
    for (const m of line.matchAll(/(?<!`)(`+)(?!`)(.+?)(?<!`)\1(?!`)/g)) out.push(m[2].trim());
    return out;
}

// ── The code tree ───────────────────────────────────────────────────────────

/** Tracked files of `root`, as repo-relative paths with forward slashes. */
export function listFiles(root) {
    const r = spawnSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard"], { cwd: root, maxBuffer: 1 << 28 });
    if (r.status === 0 && r.stdout.length > 0) {
        return [...new Set(r.stdout.toString().split("\0"))].filter((p) => p && existsSync(join(root, p)));
    }
    const out = [];
    const walk = (dir) => {
        for (const name of readdirSync(dir)) {
            if (name === ".git" || name === "node_modules" || name === "target") continue;
            const full = join(dir, name);
            if (statSync(full).isDirectory()) walk(full);
            else out.push(relative(root, full).split(sep).join("/"));
        }
    };
    walk(root);
    return out;
}

/** What the checks ask of the code: which paths exist, and which names are in which file. */
export function codeIndex(root, files = listFiles(root)) {
    const fileSet = new Set(files);
    const dirSet = new Set();
    for (const f of files) {
        for (let i = f.indexOf("/"); i > 0; i = f.indexOf("/", i + 1)) dirSet.add(f.slice(0, i));
    }
    const roots = new Set([...fileSet, ...dirSet].filter((p) => !p.includes("/")));
    const tokensOf = new Map();
    const fileTokens = (path) => {
        let set = tokensOf.get(path);
        if (!set) {
            set = new Set(readFileSync(join(root, path), "utf8").match(/[A-Za-z_][A-Za-z0-9_]*/g) ?? []);
            tokensOf.set(path, set);
        }
        return set;
    };
    let all = null;
    const allTokens = () => {
        if (!all) {
            all = new Set();
            for (const f of files) {
                if (!TEXT_EXT.test(f) || f.startsWith("docs/")) continue;
                for (const t of fileTokens(f)) all.add(t);
                tokensOf.delete(f);
            }
        }
        return all;
    };
    return { root, files, fileSet, dirSet, roots, fileTokens, allTokens };
}

// ── The checks ──────────────────────────────────────────────────────────────

const IDENT = /^(?:[A-Za-z_]\w*::)*[A-Za-z_]\w*(?:\(\))?$/;

/** The last name of an identifier-like span if it is specific enough to look up, else null. */
export function lookupName(span) {
    if (!IDENT.test(span)) return null;
    const call = span.endsWith("()");
    const name = span.replace(/\(\)$/, "").split("::").pop();
    const path = span.includes("::");
    const snake = name.includes("_") && name.length >= 5 && /[A-Za-z]/.test(name);
    const camel = /^[A-Za-z][a-z0-9]+(?:[A-Z][a-z0-9]*)+$|^[A-Z][a-z0-9]+(?:[A-Z][a-z0-9]*)+$/.test(name) && name.length >= 5;
    const upper = /^[A-Z][A-Z0-9_]{4,}$/.test(name);
    return call || path || snake || camel || upper ? name : null;
}

/** `{ path, line }` if a span is a path into the code (starting at one of its top-level names), else null. */
export function pathClaim(span, index) {
    const cut = span.indexOf("::");
    const head = cut > 0 && /\.\w+$/.test(span.slice(0, cut)) ? span.slice(0, cut) : span;
    const lined = LINE_SPAN.exec(head);
    const path = (lined ? lined[1] : head).replace(/^\.\//, "");
    const first = path.split("/")[0];
    if (!path.includes("/") || (!index.roots.has(first) && first !== "specs")) return null;
    if (PLACEHOLDER.test(path) || /\s/.test(path)) return null;
    return { path, line: Boolean(lined), symbol: head === span ? null : span.slice(head.length + 2) };
}

function exists(index, path) {
    const clean = path.replace(/\/$/, "");
    return index.fileSet.has(clean) || index.dirSet.has(clean);
}

function hint(path, index) {
    if (path.startsWith("specs/") && index.fileSet.has(`docs/${path}`)) return ` (did you mean docs/${path}?)`;
    return "";
}

/**
 * Findings for one doc: `{ kind, text, line, message }`. `text` identifies the finding in
 * the baseline; the same text twice in one doc is one finding.
 */
export function checkDoc(text, index, { allow = new Set(), deny = [], anySymbol = false } = {}) {
    const found = [];
    const lines = proseLines(text);
    const add = (kind, key, n, message) => found.push({ kind, text: key, line: n, message });

    for (const { n, text: line } of lines) {
        const spans = codeSpans(line);
        const files = [];
        const names = [];
        for (const span of spans) {
            if (allow.has(span)) continue;
            const claim = pathClaim(span, index);
            if (claim) {
                if (claim.line) add("line-citation", span, n, `\`${span}\` cites a line number, which drifts with the next edit: cite the symbol`);
                if (!exists(index, claim.path)) add("missing-path", claim.path, n, `\`${claim.path}\` is not in the code${hint(claim.path, index)}`);
                else if (index.fileSet.has(claim.path)) files.push(claim.path);
                if (claim.symbol) names.push({ span: claim.symbol, from: span });
                continue;
            }
            const bare = LINE_SPAN.exec(span);
            if (bare) {
                add("line-citation", span, n, `\`${span}\` cites a line number, which drifts with the next edit: cite the symbol`);
                continue;
            }
            const name = lookupName(span);
            if (name) names.push({ span, name });
        }
        if (LINE_PROSE.test(line) && !spans.some((s) => LINE_SPAN.test(s))) {
            const m = LINE_PROSE.exec(line)[0];
            add("line-citation", m.slice(0, 60), n, `"${m}" cites a line number, which drifts with the next edit: cite the symbol`);
        }
        const readable = files.filter((f) => TEXT_EXT.test(f));
        for (const item of names) {
            const name = item.name ?? lookupName(item.span) ?? item.span;
            if (allow.has(name) || allow.has(item.span)) continue;
            // With no file on the line, a name is looked up in the whole code only when asked to
            // (the docs of AgentMux itself name Chromium's and CEF's identifiers, which are not here).
            if (readable.length === 0 && !anySymbol) continue;
            const where = readable.length > 0 ? readable.some((f) => index.fileTokens(f).has(name)) : index.allTokens().has(name);
            if (!where) {
                add(
                    "missing-symbol",
                    item.span,
                    n,
                    readable.length > 0 ? `\`${item.span}\` is not in ${readable.join(" or ")}` : `\`${item.span}\` is not in the code`
                );
            }
        }
    }
    // A denied term is wrong in a command in a code block as much as in a sentence.
    if (deny.length) {
        text.split("\n").forEach((line, i) => {
            for (const { term, say } of deny) {
                if (line.includes(term)) add("denied-term", term, i + 1, `"${term}" is not to be used: ${say}`);
            }
        });
    }
    return [...tableFindings(lines, index, allow), ...found];
}

/** `term => what to say instead` per line; `#` comments and blank lines ignored. */
export function parseDeny(text) {
    const out = [];
    for (const raw of text.split("\n")) {
        const line = raw.trim();
        if (!line || line.startsWith("#")) continue;
        const [term, ...rest] = line.split("=>");
        if (!term.trim()) continue;
        out.push({ term: term.trim(), say: rest.join("=>").trim() || "see the code" });
    }
    return out;
}

/** One term per line; `#` comments and blank lines ignored. */
export function parseAllow(text) {
    return new Set(text.split("\n").map((l) => l.trim()).filter((l) => l && !l.startsWith("#")));
}

// ── File tables ─────────────────────────────────────────────────────────────

function isTestFile(path) {
    return CODE_EXT.test(path) && !isSourcePath(path);
}

/** The cells of a table row, without the outer pipes. */
function cells(line) {
    return line.trim().replace(/^\|/, "").replace(/\|\s*$/, "").split(/(?<!\\)\|/).map((c) => c.trim());
}

/**
 * Under a heading that names a folder in backticks, the first table lists the folder's
 * files: findings for a file the table lacks and a row that is not a file there.
 */
function tableFindings(lines, index, allow) {
    const found = [];
    for (let i = 0; i < lines.length; i++) {
        const heading = /^#{1,6}\s/.test(lines[i].text) ? lines[i] : null;
        if (!heading) continue;
        const folder = codeSpans(heading.text)
            .map((s) => s.replace(/^\.\//, ""))
            .find((s) => s.endsWith("/") && index.dirSet.has(s.replace(/\/$/, "")));
        if (!folder) continue;
        let j = i + 1;
        while (j < lines.length && !/^#{1,6}\s/.test(lines[j].text) && !lines[j].text.trim().startsWith("|")) j++;
        if (j >= lines.length || /^#{1,6}\s/.test(lines[j].text)) continue;
        const rows = [];
        for (; j < lines.length && lines[j].text.trim().startsWith("|"); j++) {
            if (/^\|?\s*:?-{2,}/.test(lines[j].text.trim())) continue;
            rows.push(lines[j]);
        }
        const listed = new Map();
        for (const row of rows.slice(1)) {
            const first = cells(row.text)[0] ?? "";
            const name = (codeSpans(first)[0] ?? first).replace(/^\.\//, "");
            if (name) listed.set(name.replace(/^.*\//, ""), { name, n: row.n });
        }
        const dir = folder.replace(/\/$/, "");
        const direct = index.files
            .filter((f) => f.startsWith(`${dir}/`) && !f.slice(dir.length + 1).includes("/") && !isTestFile(f))
            .map((f) => f.slice(dir.length + 1));
        for (const file of direct) {
            if (!listed.has(file) && !allow.has(`${dir}/${file}`)) {
                found.push({
                    kind: "table-missing-file",
                    text: `${dir}/${file}`,
                    line: heading.n,
                    message: `\`${dir}/${file}\` is in the folder but not in the table under "${heading.text.trim()}"`,
                });
            }
        }
        for (const [base, { name, n }] of listed) {
            const full = name.includes("/") ? `${dir}/${name}` : `${dir}/${base}`;
            if (!/\.\w+$/.test(base) || allow.has(name)) continue;
            if (!index.fileSet.has(full) && !index.dirSet.has(full)) {
                found.push({ kind: "table-unknown-file", text: full, line: n, message: `\`${name}\` is a row of the table under "${heading.text.trim()}" but is not in ${dir}/` });
            }
        }
    }
    return found;
}

// ── Running ─────────────────────────────────────────────────────────────────

/** The Markdown files under each of `targets` (files or folders, relative to `base`). */
export function docFiles(base, targets) {
    const out = [];
    const walk = (full, rel) => {
        const st = statSync(full);
        if (st.isDirectory()) {
            for (const name of readdirSync(full)) {
                if (name === "node_modules" || name === ".git") continue;
                walk(join(full, name), rel ? `${rel}/${name}` : name);
            }
        } else if (/\.mdx?$/.test(rel)) out.push(rel);
    };
    for (const t of targets) {
        const full = join(base, t);
        if (existsSync(full)) walk(full, t.replace(/\\/g, "/").replace(/\/$/, ""));
    }
    return out;
}

/** The findings of every doc: a Map from `<doc>::<kind>::<text>` to `{ doc, kind, text, line, message }`. */
export function checkDocs(docs, base, index, options) {
    const out = new Map();
    for (const doc of docs) {
        for (const f of checkDoc(readFileSync(join(base, doc), "utf8"), index, options)) {
            const key = `${doc}::${f.kind}::${f.text}`;
            if (!out.has(key)) out.set(key, { doc, ...f });
        }
    }
    return out;
}

// ── The baseline file ───────────────────────────────────────────────────────

const byText = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

export function parseBaseline(text) {
    return new Set(text.split("\n").map((l) => l.trim()).filter((l) => l && !l.startsWith("#")));
}

export function formatBaseline(keys, file) {
    return (
        `# Findings of ${file} that are not fixed yet, one per line (<doc>::<kind>::<text>).\n` +
        `# An entry may only be removed: run \`node scripts/check-doc-references.mjs --update\`\n` +
        `# after fixing some. A finding that is not here fails the check.\n` +
        [...keys].sort(byText).join("\n") +
        "\n"
    );
}

/** `added`: findings the baseline lacks. `fixed`: baseline entries that are no longer findings, for the docs in `touched` (all when null). */
export function compareFindings(now, baseline, touched = null) {
    const added = [...now.keys()].filter((k) => !baseline.has(k)).sort(byText);
    const fixed = [...baseline].filter((k) => !now.has(k) && (!touched || touched.has(k.split("::")[0]))).sort(byText);
    return { added, fixed };
}

// ── Main ────────────────────────────────────────────────────────────────────

function arg(name) {
    const out = [];
    const a = process.argv;
    for (let i = 0; i < a.length; i++) if (a[i] === name && a[i + 1]) out.push(a[i + 1]);
    return out;
}

function changedDocs(base) {
    const git = (args) => spawnSync("git", args, { cwd: base, maxBuffer: 1 << 26 });
    const mb = git(["merge-base", "HEAD", "origin/main"]);
    if (mb.status !== 0) return null;
    const changed = new Set();
    for (const args of [["diff", "--name-only", "-z", mb.stdout.toString().trim()], ["ls-files", "-z", "--others", "--exclude-standard"]]) {
        const r = git(args);
        if (r.status !== 0) return null;
        for (const p of r.stdout.toString().split("\0")) if (p) changed.add(p);
    }
    return changed;
}

if (isMain(import.meta.url)) {
    const code = arg("--code")[0] ?? REPO_ROOT;
    const customDocs = arg("--docs");
    const base = customDocs.length ? process.cwd() : REPO_ROOT;
    const targets = customDocs.length ? customDocs : DEFAULT_DOCS;
    const all = docFiles(base, targets);
    const docs = customDocs.length ? all : all.filter(isLivingDoc);
    const index = codeIndex(code);
    const denyFile = arg("--deny")[0];
    const allowFile = arg("--allow")[0];
    const options = {
        deny: denyFile ? parseDeny(readFileSync(denyFile, "utf8")) : [],
        allow: allowFile ? parseAllow(readFileSync(allowFile, "utf8")) : new Set(),
        anySymbol: process.argv.includes("--any-symbol"),
    };
    const now = checkDocs(docs, base, index, options);
    if (process.argv.includes("--facts") && customDocs.length) {
        // Counts, ranges and lists the page states and the code can give (lib/doc-facts.mjs).
        const dir = customDocs[0].replace(/\\/g, "/").replace(/\/$/, "");
        for (const f of checkFacts(join(base, dir), code, index.files)) {
            const doc = `${dir}/${f.page}`;
            const key = `${doc}::${f.kind}::${f.text}`;
            if (!now.has(key)) now.set(key, { doc, kind: f.kind, text: f.text, line: f.line, message: f.message });
        }
    }
    const baselineFile = arg("--baseline")[0];

    const report = (keys) => {
        for (const k of keys) {
            const f = now.get(k);
            console.error(f ? `  ${f.doc}:${f.line}: [${f.kind}] ${f.message}` : `  ${k}`);
        }
        console.error();
    };

    if (process.argv.includes("--init")) {
        if (!baselineFile) throw new Error("--init needs --baseline <file>");
        writeFileSync(baselineFile, formatBaseline(now.keys(), baselineFile));
        console.log(`check-doc-references: wrote ${baselineFile} (${now.size} findings).`);
        process.exit(0);
    }

    const baseline = baselineFile ? parseBaseline(readFileSync(baselineFile, "utf8")) : new Set();
    if (process.argv.includes("--update")) {
        if (!baselineFile) throw new Error("--update needs --baseline <file>");
        const kept = [...baseline].filter((k) => now.has(k));
        writeFileSync(baselineFile, formatBaseline(kept, baselineFile));
        console.log(`check-doc-references: wrote ${baselineFile} (${kept.length} findings).`);
        process.exit(0);
    }

    const touched = baselineFile ? changedDocs(base) : null;
    const { added, fixed } = compareFindings(now, baseline, touched);
    if (added.length) {
        console.error(`ERROR: ${added.length} reference(s) in the docs do not match the code${baselineFile ? " and are not in the baseline" : ""}. Fix the doc (the path, the symbol, or cite no line number):`);
        report(added);
    }
    if (fixed.length) {
        console.error(`ERROR: ${baselineFile} is out of date. These findings are fixed (thank you):`);
        report(fixed);
        console.error(`Run \`node scripts/check-doc-references.mjs --update --baseline ${baselineFile}\` and commit the baseline.`);
    }
    if (added.length || fixed.length) process.exit(1);
    console.log(`check-doc-references: ok. ${docs.length} docs, ${now.size} known finding(s) in the baseline, none new.`);
}
