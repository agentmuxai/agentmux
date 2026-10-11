#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-comment-hygiene.mjs — keeps review history out of code comments, and
// proves a comment-only condensing PR changed no code.
// Spec: docs/specs/SPEC_CODE_COMMENT_DENSITY_AND_CONDENSING_2026_09_30.md §6, §7.3.
//
//   node scripts/check-comment-hygiene.mjs                  gate (what CI runs)
//   node scripts/check-comment-hygiene.mjs --report         comment density, repo-wide
//   node scripts/check-comment-hygiene.mjs --dead-refs [--json]
//                                                           every comment naming a missing file
//   node scripts/check-comment-hygiene.mjs --code-equal <base>
//                                                           code must be identical to <base>
//
// GATE. Looks only at comment text on lines this branch ADDS (diff against the
// merge-base with the PR base, working tree included). It fails on review-history
// narration: a severity tag beside a PR number, "re-review", "round N", or a
// review bot named next to a PR number or a verdict. That text belongs in the
// PR thread, and `(#NNNN)` is the whole citation. It is scoped to added lines
// on purpose: a repo-wide gate would fail on the existing backlog and be
// switched off (the same reason scripts/check-spec-citations.sh is scoped to
// changed files). A comment the same diff also removes elsewhere (a split, a
// move to another file, a re-indent) is not new and is not held to the rule, so
// a move-only PR passes. It also prints two warnings that never fail the build:
//   - a new `//` or `/* */` block longer than 8 lines (rule C3; doc comments
//     are exempt, an interface doc may be long);
//   - a file already above 40% comment lines whose ratio this branch raised.
// A line can opt out with `comment-hygiene: allow` in the comment, for the rare
// case where the marker words are the subject (e.g. documenting the review bot).
//
// DEAD FILE REFERENCES (rule C7), also part of the gate:
//   - An added comment that names a file not in the repo fails when the name
//     is a doc (`SPEC_…`, `docs/…`) or a repo-rooted path (`crates/…`,
//     `frontend/…`); a bare name (`persistent.rs`) or partial path only warns. (comment-hygiene: allow)
//     The split follows a triage of every dead reference on main: repo paths
//     were 95% genuinely stale and doc names 72%, bare names and partial paths
//     under 60% (property accesses, other repos, history).
//   - A file this branch deletes or renames away fails every comment anywhere
//     that still names it, so a split updates its pointers in the same PR.
//     Only names that no longer exist anywhere count (`mod.rs` leaving one
//     directory proves nothing). This is what left 67 comments pointing at
//     `persistent.rs` and `bootstrap.rs` after their splits. (comment-hygiene: allow)
//   Comments are read in .ts/.tsx/.rs, the JavaScript family and stylesheets;
//   the narration and density rules above stay on .ts/.tsx/.rs. Shell,
//   PowerShell (`#` comments) and Markdown are not read.
//   Runtime files AgentMux writes (CLAUDE.md, ...), sibling repos and the old
//   `src-tauri/` tree are not repo files and are skipped. So is a bare name the
//   file's own code uses (a property access, a test's fixture, a file name read
//   at runtime). The same opt-out applies, for a deliberately historical
//   or illustrative name; a historical one usually reads better naming the
//   module (`the single-file bootstrap module`) than a file that is gone.
//
// CODE-EQUAL. For every .ts/.tsx/.rs file that differs from the merge-base with
// <base>, strips the comments and compares the rest, with whitespace outside
// string literals collapsed. Any code change fails, including inside a string.
// It also requires the per-file counts of SAFETY:, TODO, FIXME, lint and
// compiler directives and doctest fences to be unchanged (rule C8), and lists
// each `#NNNN` / `SPEC_...` citation the comments dropped, so a reviewer can
// confirm each one was narration or a duplicate. Use it on a comment-only PR.
// Only .ts/.tsx/.rs files are compared; any other file the branch changes is
// listed as a warning, not verified.
//
// LIMITS of the lexer (shared by all modes). It is not a parser. JSX text that
// contains `//` or an apostrophe, and a regex literal after an unusual token,
// can be misread; the effect is a comment missed or a code line counted as
// comment, never a crash.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

// The repo root, found from the working directory so the pure functions below
// import cleanly under vitest (where import.meta.url is not a file: URL).
let rootDir = null;
function repoRoot() {
    rootDir ||= execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
    return rootDir;
}
const ALLOW_TOKEN = "comment-hygiene: allow";
const DENSITY_WARN_RATIO = 0.4;
const LONG_BLOCK_LINES = 8;
const SOURCE_RE = /\.(?:tsx?|rs)$/;
const SKIP_RE = /(^|\/)(node_modules|target|dist|build)\/|^frontend\/types\/rpc\/|(^|\/)splash_font\.rs$/;
const TEST_RE = /\.(?:test|spec)\.tsx?$|(^|\/)(?:tests?|__tests__)\/|_tests?\.rs$|\/tests\.rs$/;

export const isSourcePath = (p) => SOURCE_RE.test(p) && !SKIP_RE.test(p);
// Files whose comments the reference checks read: source plus the JavaScript
// family (scripts, tools) and stylesheets, which use the same `//` and `/* */`
// syntax. Shell and PowerShell (`#` comments) and Markdown are not read; docs
// have their own link gates (check-doc-links.mjs, check-spec-citations.sh).
const SCAN_GLOBS = ["*.ts", "*.tsx", "*.rs", "*.js", "*.jsx", "*.mjs", "*.cjs", "*.scss", "*.css"];
const SCAN_RE = /\.(?:tsx?|rs|jsx?|mjs|cjs|s?css)$/;
export const isScanPath = (p) => SCAN_RE.test(p) && !SKIP_RE.test(p);
export const isTestPath = (p) => TEST_RE.test(p);
export const langOf = (p) => (p.endsWith(".rs") ? "rs" : "ts");

const SQ = "'";
const BT = "`";
const IDENT = /[A-Za-z0-9_$]/;
// A `/` after one of these starts a regex literal; after an identifier, `)` or
// `]` it is division. `<` and `>` are left out so JSX `</div>` is not a regex.
const REGEX_AFTER_CHARS = "(,=:[!&|?{;+-*%~^";
const REGEX_AFTER_WORDS = new Set(["return", "typeof", "case", "do", "else", "in", "of", "void", "delete", "throw", "yield", "await"]);

/**
 * Splits a Rust or TypeScript source into comments and code.
 *
 * Returns `lines` (one entry per source line: `code` is true when the line has
 * a non-comment, non-blank character; `text` is the comment text on it;
 * `kinds` is a subset of "doc", "line", "block") and `code`, the source with
 * every comment replaced by whitespace and whitespace outside string literals
 * collapsed (to one space, or a newline if the run held one), for comparing two
 * versions of a file.
 */
export function lexSource(src, lang) {
    const rust = lang === "rs";
    const n = src.length;
    const lines = [];
    let line = 0;
    const cur = () => (lines[line] ||= { code: false, text: "", kinds: new Set() });
    const out = [];
    // Whitespace between tokens is pending until the next token: " ", or "\n" if
    // the run held a line terminator (a newline, or a block comment spanning
    // lines, which JavaScript treats as one). Line terminators stay significant
    // because automatic semicolon insertion makes `return\nx` differ from
    // `return x`.
    let ws = null;
    let prevSig = "";
    let prevTwo = "";
    let word = "";

    const flushWs = () => {
        if (ws && out.length) out.push(ws);
        ws = null;
    };
    const space = (nl) => {
        ws = nl || ws === "\n" ? "\n" : " ";
    };
    const code = (ch) => {
        if (/\s/.test(ch)) return space(false);
        flushWs();
        out.push(ch);
        cur().code = true;
        prevSig = ch;
        prevTwo = (prevTwo + ch).slice(-2);
        word = IDENT.test(ch) ? word + ch : "";
    };
    const str = (ch) => {
        flushWs();
        out.push(ch);
        cur().code = true;
        prevSig = '"';
        prevTwo = "";
        word = "";
    };
    const comment = (ch, kind) => {
        const l = cur();
        l.text += ch;
        l.kinds.add(kind);
        space(false);
    };
    const newline = (asStr) => {
        if (asStr) str("\n");
        else space(true);
        line++;
    };

    // Template literals: the text parts are strings, but a `${ ... }` substitution
    // is code (it can hold comments). `tmplStack` holds, for each open
    // substitution, the number of `{` opened inside it and not yet closed.
    const tmplStack = [];
    // Scans template text from `j` until the closing backtick or the next `${`;
    // returns the index to continue from.
    const templateBody = (j) => {
        while (j < n) {
            const ch = src[j];
            if (ch === "\\") {
                str(ch);
                j++;
                if (j < n) {
                    if (src[j] === "\n") newline(true);
                    else str(src[j]);
                    j++;
                }
            } else if (ch === BT) {
                str(ch);
                return j + 1;
            } else if (ch === "$" && src[j + 1] === "{") {
                str("$");
                str("{");
                tmplStack.push(0);
                return j + 2;
            } else {
                if (ch === "\n") newline(true);
                else str(ch);
                j++;
            }
        }
        return j;
    };

    let i = 0;
    while (i < n) {
        const c = src[i];
        const d = src[i + 1];
        if (c === "\n") {
            newline(false);
            i++;
            continue;
        }
        if (c === "/" && d === "/") {
            const e = src[i + 2];
            const kind = rust && ((e === "/" && src[i + 3] !== "/") || e === "!") ? "doc" : "line";
            while (i < n && src[i] !== "\n") comment(src[i++], kind);
            continue;
        }
        if (c === "/" && d === "*") {
            const kind = (src[i + 2] === "*" && src[i + 3] !== "/") || (rust && src[i + 2] === "!") ? "doc" : "block";
            let depth = 0;
            while (i < n) {
                if (src[i] === "/" && src[i + 1] === "*") {
                    depth = rust ? depth + 1 : 1;
                    comment("/", kind);
                    comment("*", kind);
                    i += 2;
                } else if (src[i] === "*" && src[i + 1] === "/") {
                    comment("*", kind);
                    comment("/", kind);
                    i += 2;
                    depth--;
                    if (depth <= 0) break;
                } else if (src[i] === "\n") {
                    space(true);
                    line++;
                    i++;
                } else {
                    comment(src[i++], kind);
                }
            }
            continue;
        }
        if (rust && c === "r" && (d === '"' || d === "#") && !/[A-Za-z0-9_]/.test(src[i - 1] === "b" ? " " : src[i - 1] || " ")) {
            let j = i + 1;
            let hashes = 0;
            while (src[j] === "#") {
                hashes++;
                j++;
            }
            if (src[j] === '"') {
                const end = '"' + "#".repeat(hashes);
                for (let k = i; k <= j; k++) str(src[k]);
                i = j + 1;
                while (i < n && !src.startsWith(end, i)) {
                    if (src[i] === "\n") newline(true);
                    else str(src[i]);
                    i++;
                }
                for (let k = 0; k < end.length && i < n; k++) str(src[i++]);
                continue;
            }
        }
        if (c === '"' || (!rust && c === SQ)) {
            str(c);
            i++;
            while (i < n && src[i] !== c) {
                if (src[i] === "\\" && i + 1 < n) {
                    str(src[i++]);
                }
                if (src[i] === "\n") {
                    // A TS quoted literal cannot hold a raw newline, so an open one
                    // is JSX text (an apostrophe, `6"`): stop at the line end
                    // instead of swallowing the comments below it.
                    if (!rust) break;
                    newline(true);
                } else {
                    str(src[i]);
                }
                i++;
            }
            if (i < n && src[i] === c) str(src[i++]);
            continue;
        }
        if (!rust && c === BT) {
            str(c);
            i = templateBody(i + 1);
            continue;
        }
        if (!rust && tmplStack.length > 0 && (c === "{" || c === "}")) {
            // Inside a `${ ... }` substitution: count braces to find its end,
            // then resume the template's string text.
            const top = tmplStack.length - 1;
            if (c === "{") {
                tmplStack[top]++;
            } else if (tmplStack[top] === 0) {
                tmplStack.pop();
                str(c);
                i = templateBody(i + 1);
                continue;
            } else {
                tmplStack[top]--;
            }
        }
        if (rust && c === SQ) {
            if (d === "\\") {
                let j = i + 3;
                while (j < n && src[j] !== SQ && src[j] !== "\n") j++;
                for (let k = i; k <= j && k < n; k++) str(src[k]);
                i = j + 1;
                continue;
            }
            if (src[i + 2] === SQ) {
                str(c);
                str(d);
                str(src[i + 2]);
                i += 3;
                continue;
            }
            code(c);
            i++;
            continue;
        }
        if (!rust && c === "/" && (prevSig === "" || prevTwo === "=>" || REGEX_AFTER_CHARS.includes(prevSig) || (IDENT.test(prevSig) && REGEX_AFTER_WORDS.has(word)))) {
            let j = i + 1;
            let inClass = false;
            let ok = false;
            while (j < n && src[j] !== "\n") {
                if (src[j] === "\\") j++;
                else if (src[j] === "[") inClass = true;
                else if (src[j] === "]") inClass = false;
                else if (src[j] === "/" && !inClass) {
                    ok = j > i + 1;
                    break;
                }
                j++;
            }
            if (ok) {
                for (let k = i; k <= j; k++) str(src[k]);
                i = j + 1;
                continue;
            }
        }
        code(c);
        i++;
    }
    for (const l of lines) if (l) l.text = l.text.replace(/\r$/, "");
    const filled = [];
    const total = src.length === 0 ? 0 : src.split("\n").length;
    for (let k = 0; k < total; k++) filled.push(lines[k] || { code: false, text: "", kinds: new Set() });
    return { lines: filled, code: out.join("").trim() };
}

const isCommentOnly = (l) => l.text.trim() !== "" && !l.code;

/** Line counts for a lexed file. */
export function statsOf(info) {
    let commentOnly = 0;
    let commentChars = 0;
    for (const l of info.lines) {
        commentChars += l.text.trim().length;
        if (isCommentOnly(l)) commentOnly++;
    }
    return { lines: info.lines.length, commentOnly, commentChars, ratio: info.lines.length ? commentOnly / info.lines.length : 0 };
}

// Review-history narration. Every pattern needs a review artifact (a severity
// tag, a PR number, a round count, a bot named with a verdict); a bare mention
// of the review bot as a product does not match.
const NARRATION = [
    // Uppercase P only: `p1`/`p2` are point variables, and `#333` is a colour.
    ["severity tag next to a PR number", /\bP[0-3]\b.{0,40}#\d{3,}|#\d{3,}.{0,40}\bP[0-3]\b/],
    ["re-review", /\bre-?review/i],
    ["review round count", /\bround \d+\b/i],
    ["review bot next to a PR number", /\b(?:reagentx?|muxreview|codex)\b[^.\n]{0,20}#\d{3,}/i],
    ["review bot with a severity tag", /\b(?:reagentx?|ReAgent|muxreview|codex|Codex)\b.{0,12}\bP[0-3]\b/],
    ["review bot verdict", /\b(?:reagentx?|muxreview|codex)\s+(?:asked|flagged|caught|found|noted|pointed|raised|requested|suggested)\b/i],
];

/** Returns the rule name for review-history narration in `text`, or null. */
export function narrationRule(text) {
    if (text.includes(ALLOW_TOKEN)) return null;
    for (const [name, re] of NARRATION) if (re.test(text)) return name;
    return null;
}

// ------------------------------------------------------- file references

// File names a comment can point at. JSON/TOML/YAML are left out: in comments
// they are almost always files AgentMux or a CLI reads at runtime, not files in
// this repo.
const REF_EXT = "rs|tsx?|jsx?|mjs|cjs|sh|ps1|scss|css|md";
const REF_FILE = new RegExp(`\\.(?:${REF_EXT})$`);
// commentRefs drops a name that is the tail of a glob (`Attachment*Event.ts`,
// a word right before the `*`) or of an octal escape (`Caf\303\251.md`). A
// Markdown-emphasised name (`*src/x.rs*`) and a Windows path (`frontend\app\x.ts`) (comment-hygiene: allow)
// are still read.
const REF_RE = new RegExp(`(?<![\\w./@-])((?:\\.{1,2}/)*(?:[\\w@.-]+/)*[\\w@-][\\w@.-]*\\.(?:${REF_EXT}))(?![\\w/-])`, "g");
const OCTAL_ESCAPE_TAIL = /^[0-7]{3}\./;
// Names that are real but not files in this repo: what AgentMux writes into an
// agent's workdir or reads from a provider CLI's config, sibling repos, and the
// pre-CEF `src-tauri/` tree that "ported from" comments cite. Measured against a
// full triage of the tree's dead references (REPORT_COMMENT_COMPRESSION_WORTH_IT_2026_10_02.md §6).
// Also: package paths (`vite/…`, `shiki/…`), the xterm.js repository, and a
// crate's versioned source folder in the cargo registry (`keyring-2.3.3/…`).
const FOREIGN_REF =
    /^(?:CLAUDE|CLAUDE\.local|CLAUDE_CONTAINER|AGENTS|GEMINI|QWEN|KIMI|MEMORY|AGENTMUX_MEMORY|SKILL|SYSTEM|APPEND_SYSTEM|copilot-instructions)\.md$/;
// Generic names that are foreign only on their own (a bundle's `notes.md`, a
// provider's `system_prompt.md`, the TypeScript compiler's `lib.*.d.ts`): with a
// folder in front (`docs/notes.md`) they name a repo file and are checked. (comment-hygiene: allow)
const FOREIGN_BARE = /^(?:(?:system_prompt|notes)\.md|lib(?:\.[\w-]+)+\.d\.ts)$/;
const FOREIGN_PREFIX =
    /^(?:\.claude|\.codex|\.gemini|\.qwen|\.pi|\.copilot|\.agentmux|\.github\/instructions|~|HOME|agentmux-cloud|agentmux-ai|muxbus|reagent|src-tauri|node_modules|vite|shiki|xtermjs|[\w-]+-\d+\.\d+\.\d+)\//;
const DOC_NAME = /^(?:SPEC|REPORT|PLAN|RUNBOOK|RETRO|ADR|PRD|AUDIT)_[A-Za-z0-9_.-]+\.md$/;

const baseName = (p) => p.slice(p.lastIndexOf("/") + 1);

/** A path with `./`, `../` prefixes and `/./` segments removed. */
export function cleanRef(ref) {
    return ref.replace(/^(?:\.{1,2}\/)+/, "").replace(/\/\.\//g, "/");
}

/**
 * File references in a lexed file's comments: `[{ line, ref }]`, 1-based lines.
 * A name hard-wrapped across two comment lines (`SPEC_FOO_` / `2026_01_01.md`) (comment-hygiene: allow)
 * is joined back together.
 */
export function commentRefs(info) {
    const refs = [];
    info.lines.forEach((l, idx) => {
        if (!l.text) return;
        const body = normalizeComment(l.text);
        for (const m of body.matchAll(REF_RE)) {
            let ref = m[1];
            const before = body[m.index - 1];
            if (before === "\\" && OCTAL_ESCAPE_TAIL.test(ref)) continue;
            if (before === "*" && /\w/.test(body[m.index - 2] || "")) continue;
            const prev = idx > 0 ? normalizeComment(info.lines[idx - 1].text) : "";
            if (m.index === 0 && prev) {
                const tail = /[\w@./-]+$/.exec(prev);
                // Only a real mid-name break: the previous line ends in a word
                // character plus `_`/`-` (`SPEC_FOO_`), or this piece starts with
                // a digit (`2026_01_01.md`). An SCSS partial (`_x.scss`) or a (comment-hygiene: allow)
                // `--` separator is not a continuation.
                if (tail && (/^_?\d/.test(ref) || /[A-Za-z0-9][_-]$/.test(tail[0]))) ref = tail[0] + ref;
            }
            // `a.ts/b.ts` is a list of two names, not a path: a directory (comment-hygiene: allow)
            // segment never carries a source extension.
            const segs = ref.split("/");
            const parts = segs.slice(0, -1).some((s) => REF_FILE.test(s)) ? segs.filter((s) => REF_FILE.test(s)) : [ref];
            for (const part of parts) refs.push({ line: idx + 1, ref: part, text: l.text });
        }
    });
    return refs;
}

/** What a resolver needs to know about the tracked tree. */
export function buildRefIndex(paths) {
    const tops = new Set();
    for (const p of paths) if (p.includes("/")) tops.add(p.slice(0, p.indexOf("/")));
    return { paths: new Set(paths), list: paths, bases: new Set(paths.map(baseName)), tops };
}

/** `doc` (a docs/ file or a SPEC_/REPORT_/... name), `path` (repo-rooted), or `bare`. */
export function refKind(ref, index) {
    const r = cleanRef(ref);
    if (DOC_NAME.test(baseName(r)) || r.startsWith("docs/")) return "doc";
    if (r.includes("/") && index.tops.has(r.slice(0, r.indexOf("/")))) return "path";
    return "bare";
}

/** True when `ref` names something outside this repo on purpose. */
export function isForeignRef(ref) {
    const r = cleanRef(ref);
    // The prefix is tested after `./` and `../` are stripped, so
    // `../agentmux-cloud/docs/X.md` is recognised as a sibling repo.
    // A bare `x.js` is almost always a library (`xterm.js`, `Node.js`) in this
    // TypeScript/Rust tree; a `.js` path is still checked, and so is a bare
    // name when its file is removed (staleAfterMove does not use this).
    const bareJs = !r.includes("/") && /\.jsx?$/.test(r);
    const bareGeneric = !r.includes("/") && FOREIGN_BARE.test(r);
    return bareJs || bareGeneric || FOREIGN_REF.test(baseName(r)) || FOREIGN_PREFIX.test(r) || r.includes("...") || r.includes("*");
}

/** True when some tracked file is what `ref` names (by full path, path suffix, or basename). */
export function resolvesRef(ref, index) {
    const r = cleanRef(ref);
    if (!r.includes("/")) return index.bases.has(r);
    if (index.paths.has(r)) return true;
    const suffix = `/${r}`;
    return index.list.some((p) => p.endsWith(suffix));
}

/**
 * True when a bare name (`kind` "bare") is a name the file's own code uses,
 * not a pointer at a repo file: a property access (`item.ts`, a timestamp (comment-hygiene: allow)
 * field), a fixture a test passes around, or a file name the code reads or
 * writes at runtime. Doc names and repo-rooted paths are never excused this way.
 */
export function isBareNonRef(ref, info) {
    const r = cleanRef(ref);
    if (!info?.code) return false;
    const escaped = r.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");
    return new RegExp(`(?<![\\w$./\\\\-])${escaped}(?![\\w$/-])`).test(info.code);
}

/** True when a comment's file name needs no attention (rule C7). */
function refIsFine(ref, info, index) {
    if (isForeignRef(ref) || resolvesRef(ref, index)) return true;
    return refKind(ref, index) === "bare" && isBareNonRef(ref, info);
}

/**
 * Dead file references on added lines of one file. An unresolved `doc` or
 * `path` reference is an error; a `bare` name only warns, because bare names
 * collide with property accesses and other repos' files (see isBareNonRef for
 * the cases that are skipped outright). Comments that only moved (see
 * gateFile) are skipped.
 */
export function deadRefFindings(info, added, index, moved = new Set()) {
    const errors = [];
    const warnings = [];
    for (const { line, ref, text } of commentRefs(info)) {
        if (!added.has(line) || text.includes(ALLOW_TOKEN)) continue;
        if (moved.has(normalizeComment(text))) continue;
        if (refIsFine(ref, info, index)) continue;
        const kind = refKind(ref, index);
        const message =
            `comment names \`${ref}\`, which is not in this repo. Point it at the current file or drop it (rule C7). ` +
            `A file in another repo: prefix it with the repo (\`agentmux-cloud/…\`). An illustrative name: add \`${ALLOW_TOKEN}\`.`;
        (kind === "bare" ? warnings : errors).push({ line, message });
    }
    return { errors, warnings };
}

/**
 * Comments anywhere that name a file this branch deleted or renamed away.
 * `gone` is `[{ path, to }]` (`to` is null for a deletion); only paths whose
 * basename no longer exists anywhere are passed. Several removed files can
 * share a basename: a path-qualified reference is matched against each, and a
 * bare name lists all of them. `files` is `[{ file, info }]`. Returns
 * `[{ file, line, message }]`.
 */
export function staleAfterMove(gone, files) {
    const out = [];
    const byBase = new Map();
    for (const g of gone) {
        const b = baseName(g.path);
        if (!byBase.has(b)) byBase.set(b, []);
        byBase.get(b).push(g);
    }
    const describe = (g) => (g.to ? `renames \`${g.path}\` to \`${g.to}\`` : `deletes \`${g.path}\``);
    for (const { file, info } of files) {
        for (const { line, ref, text } of commentRefs(info)) {
            if (text.includes(ALLOW_TOKEN)) continue;
            const r = cleanRef(ref);
            const candidates = byBase.get(baseName(r)) || [];
            const hits = r.includes("/") ? candidates.filter((g) => g.path === r || g.path.endsWith(`/${r}`)) : candidates;
            if (!hits.length) continue;
            const what = hits.length === 1 ? `this branch ${describe(hits[0])}` : `this branch removes every file of that name: ${hits.map(describe).join("; ")}`;
            out.push({ file, line, message: `comment names \`${ref}\`, and ${what}. Update the comment in this PR (rule C7).` });
        }
    }
    return out;
}

/**
 * Added and removed line numbers per file from `git diff -U0` output:
 * `added` is keyed by new path (new-side numbers), `removed` by old path
 * (old-side numbers).
 */
export function parseDiff(diff) {
    const added = new Map();
    const removed = new Map();
    let oldFile = null;
    let newFile = null;
    let inHeader = false;
    const range = (m, startIdx) => {
        const start = Number(m[startIdx]);
        const count = m[startIdx + 1] === undefined ? 1 : Number(m[startIdx + 1]);
        return Array.from({ length: count }, (_, k) => start + k);
    };
    for (const raw of diff.split("\n")) {
        if (raw.startsWith("diff --git ")) {
            inHeader = true;
            oldFile = newFile = null;
            continue;
        }
        // File headers are only real before a hunk; inside one, a removed or
        // added line can itself start with `--` or `++`.
        if (inHeader && raw.startsWith("--- ")) {
            const p = raw.slice(4).trim();
            oldFile = p === "/dev/null" ? null : p.replace(/^a\//, "");
            continue;
        }
        if (inHeader && raw.startsWith("+++ ")) {
            const p = raw.slice(4).trim();
            newFile = p === "/dev/null" ? null : p.replace(/^b\//, "");
            if (newFile && !added.has(newFile)) added.set(newFile, new Set());
            continue;
        }
        const m = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(raw);
        if (m) {
            inHeader = false;
            if (oldFile) {
                if (!removed.has(oldFile)) removed.set(oldFile, new Set());
                for (const n of range(m, 1)) removed.get(oldFile).add(n);
            }
            if (newFile) for (const n of range(m, 3)) added.get(newFile).add(n);
        }
    }
    return { added, removed };
}

/** Added-line numbers per file (see parseDiff). */
export const parseAddedLines = (diff) => parseDiff(diff).added;

/**
 * A comment line's text with its comment markers and spacing removed, so the
 * same comment compares equal wherever it is indented or wrapped in a block.
 */
export function normalizeComment(text) {
    return text
        .replace(/^\s*(?:\/\/[/!]?|\/\*[*!]?|\*(?!\/))/, "")
        .replace(/\*\/\s*$/, "")
        .replace(/\s+/g, " ")
        .trim();
}

/**
 * Gate findings for one file. `added` is a Set of 1-based line numbers this
 * branch added. `moved` is a Set of normalized comment texts the same diff also
 * removed elsewhere: a comment that only moved (a split, a relocation, a
 * re-indent) is not new, so it is not held to the narration rule. Returns
 * `{ errors, warnings }`, each `{ line, message }`.
 */
export function gateFile(info, added, moved = new Set()) {
    const errors = [];
    const warnings = [];
    info.lines.forEach((l, idx) => {
        const ln = idx + 1;
        if (!added.has(ln) || !l.text) return;
        if (moved.has(normalizeComment(l.text))) return;
        const rule = narrationRule(l.text);
        if (rule) {
            errors.push({
                line: ln,
                message: `review-history comment (${rule}): keep the constraint, cite the PR once as (#NNNN), drop the narration. See CONTRIBUTING.md "Comments".`,
            });
        }
    });
    let run = [];
    const flush = () => {
        if (run.length > LONG_BLOCK_LINES) {
            warnings.push({
                line: run[0],
                message: `new comment block of ${run.length} lines (rule C3: constraint plus consequence in 3 lines; over ~8 lines belongs in docs/ with a pointer).`,
            });
        }
        run = [];
    };
    info.lines.forEach((l, idx) => {
        const ln = idx + 1;
        if (added.has(ln) && isCommentOnly(l) && !l.kinds.has("doc")) run.push(ln);
        else flush();
    });
    flush();
    return { errors, warnings };
}

// Items a condensing PR must not remove (spec C8). Counted in comment text only;
// code is covered by the code-equal comparison. Besides the markers the spec
// names, this covers every comment the toolchain acts on: bundler hints
// (`@vite-ignore`, `webpackIgnore`), test-runner docblocks (`@vitest-environment`),
// formatter and coverage switches, JSX pragmas, tree-shaking and legal-comment
// markers. The tracked source uses `@vite-ignore`, `@vitest-environment` and
// `prettier-ignore` today; the rest guard against the next one.
const PROTECTED = [
    ["SAFETY:", /\bSAFETY:/g],
    ["TODO", /\bTODO\b/g],
    ["FIXME", /\bFIXME\b/g],
    ["eslint directive", /\beslint-[a-z-]+/g],
    ["@ts directive", /@ts-[a-z-]+/g],
    ["/// <reference", /<reference\b/g],
    ["doctest fence", /```/g],
    ["bundler hint", /@vite-ignore|webpackIgnore|webpackChunkName|webpackMode|webpackPrefetch|webpackPreload/g],
    ["test-runner directive", /@vitest[\w-]*|@jest-[\w-]+/g],
    ["formatter or linter switch", /prettier-ignore|stylelint-(?:disable|enable)/g],
    ["coverage ignore", /\b(?:istanbul|c8|v8) ignore\b/g],
    ["JSX or refresh pragma", /@jsx\w*|@refresh\b/g],
    ["tree-shaking marker", /[@#]__(?:PURE|NO_SIDE_EFFECTS)__/g],
    ["legal comment", /@preserve|@license|\/\*!/g],
];

export function protectedCounts(info) {
    const text = info.lines.map((l) => l.text).join("\n");
    return Object.fromEntries(PROTECTED.map(([name, re]) => [name, (text.match(re) || []).length]));
}

const CITE = /#\d{3,}|SPEC_[A-Z0-9_]+/g;

export function citationsOf(info) {
    return new Set(info.lines.map((l) => l.text).join("\n").match(CITE) || []);
}

/**
 * Compares a file before and after a comment-only edit. Returns problem strings
 * (empty = ok) and the citations the edit dropped.
 */
export function compareCodeEqual(before, after, lang) {
    const a = lexSource(before, lang);
    const b = lexSource(after, lang);
    const problems = [];
    if (a.code !== b.code) problems.push("code changed (this PR must change comments only)");
    const pa = protectedCounts(a);
    const pb = protectedCounts(b);
    for (const k of Object.keys(pa)) {
        if (pa[k] !== pb[k]) problems.push(`${k} count changed ${pa[k]} -> ${pb[k]} (spec C8: never remove these)`);
    }
    const had = citationsOf(a);
    const has = citationsOf(b);
    const dropped = [...had].filter((c) => !has.has(c)).sort();
    return { problems, dropped };
}

// ---------------------------------------------------------------- git glue

function git(...args) {
    return execFileSync("git", ["-c", "core.quotepath=off", ...args], { cwd: repoRoot(), encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
}

function gitShow(rev, path) {
    try {
        return execFileSync("git", ["show", `${rev}:${path}`], { cwd: repoRoot(), encoding: "utf8", maxBuffer: 64 * 1024 * 1024, stdio: ["ignore", "pipe", "ignore"] });
    } catch {
        return null;
    }
}

function resolveBase(base) {
    for (const ref of [`origin/${base}`, base]) {
        try {
            git("rev-parse", "--verify", "--quiet", ref);
            return ref;
        } catch {
            // try the next candidate
        }
    }
    return null;
}

function mergeBaseWith(base) {
    const ref = resolveBase(base);
    if (!ref) return null;
    try {
        return git("merge-base", ref, "HEAD").trim();
    } catch {
        return null;
    }
}

const annotate = (level, file, line, message) => {
    if (process.env.GITHUB_ACTIONS) console.log(`::${level} file=${file},line=${line}::${message}`);
    else console.log(`${file}:${line}: ${level}: ${message}`);
};

function readWorking(path) {
    try {
        return readFileSync(join(repoRoot(), path), "utf8");
    } catch {
        return null;
    }
}

// ------------------------------------------------------------------- modes

/** Every file in the working tree that git knows or would add, minus deletions. */
function trackedTree() {
    const deleted = new Set(git("ls-files", "--deleted").split("\n").filter(Boolean));
    const all = git("ls-files", "--cached", "--others", "--exclude-standard").split("\n").filter(Boolean);
    return [...new Set(all)].filter((p) => !deleted.has(p));
}

/** Lexes every source file in the tree: `[{ file, info }]`. */
function lexTree(tree) {
    const out = [];
    for (const file of tree) {
        if (!isScanPath(file)) continue;
        const text = readWorking(file);
        if (text !== null) out.push({ file, info: lexSource(text, langOf(file)) });
    }
    return out;
}

function runGate() {
    const baseBranch = process.env.GITHUB_BASE_REF || "main";
    const mb = mergeBaseWith(baseBranch);
    if (!mb) {
        console.error("check-comment-hygiene: no merge base; nothing to check.");
        return 0;
    }
    const { added, removed } = parseDiff(git("diff", "-U0", "--no-color", "-M", mb, "--", ...SCAN_GLOBS));
    // Comments this diff removes (from the base version of each file) are not
    // new when they reappear: `-M` follows only whole-file renames, so a split or
    // a move into another file would otherwise count every moved line as added.
    const moved = new Set();
    for (const [oldFile, lines] of removed) {
        if (!isScanPath(oldFile)) continue;
        const before = gitShow(mb, oldFile);
        if (before === null) continue;
        const info = lexSource(before, langOf(oldFile));
        for (const ln of lines) {
            const t = info.lines[ln - 1] && normalizeComment(info.lines[ln - 1].text);
            if (t) moved.add(t);
        }
    }
    for (const f of git("ls-files", "--others", "--exclude-standard", "--", ...SCAN_GLOBS).split("\n").filter(Boolean)) {
        const text = readWorking(f);
        if (text !== null) added.set(f, new Set(Array.from({ length: text.split("\n").length }, (_, k) => k + 1)));
    }
    const tree = trackedTree();
    const index = buildRefIndex(tree);
    let errors = 0;
    let deadRefs = 0;
    let checked = 0;
    for (const [file, lines] of added) {
        if (!isScanPath(file) || lines.size === 0) continue;
        const text = readWorking(file);
        if (text === null) continue;
        checked++;
        const info = lexSource(text, langOf(file));
        // Narration and density apply to .ts/.tsx/.rs; file references to every
        // scanned file.
        const source = isSourcePath(file);
        const res = source ? gateFile(info, lines, moved) : { errors: [], warnings: [] };
        const refs = deadRefFindings(info, lines, index, moved);
        for (const e of [...res.errors, ...refs.errors]) annotate("error", file, e.line, e.message);
        for (const w of [...res.warnings, ...refs.warnings]) annotate("warning", file, w.line, w.message);
        errors += res.errors.length;
        deadRefs += refs.errors.length;
        const now = statsOf(info);
        if (source && now.ratio > DENSITY_WARN_RATIO && now.lines >= 200) {
            const old = gitShow(mb, file);
            const before = old === null ? null : statsOf(lexSource(old, langOf(file)));
            if (before && now.ratio > before.ratio + 0.005) {
                annotate("warning", file, 1, `comment lines are ${(now.ratio * 100).toFixed(0)}% of this file (was ${(before.ratio * 100).toFixed(0)}%). Report only; consider condensing.`);
            }
        }
    }
    // A file this branch deletes or renames away leaves every comment that names
    // it stale. Only basenames that no longer exist anywhere are checked, so the
    // match is unambiguous (`mod.rs` going away from one directory proves nothing).
    const gone = [];
    for (const row of git("diff", "--name-status", "-M", mb).split("\n").filter(Boolean)) {
        const [status, from, to] = row.split("\t");
        if ((status[0] === "D" || status[0] === "R") && REF_FILE.test(from) && !index.bases.has(baseName(from))) {
            gone.push({ path: from, to: status[0] === "R" ? to : null });
        }
    }
    let stale = 0;
    if (gone.length) {
        for (const s of staleAfterMove(gone, lexTree(tree))) {
            annotate("error", s.file, s.line, s.message);
            stale++;
        }
    }
    if (errors || deadRefs || stale) {
        if (errors) console.error(`\ncheck-comment-hygiene: ${errors} review-history comment line(s) added. Review history lives in the PR thread; keep the constraint and cite (#NNNN) once.`);
        if (deadRefs) console.error(`check-comment-hygiene: ${deadRefs} added comment(s) name a file that is not in the repo.`);
        if (stale) console.error(`check-comment-hygiene: ${stale} comment(s) name a file this branch deleted or renamed away.`);
        return 1;
    }
    console.log(`check-comment-hygiene: ok (${checked} changed source file(s) checked${gone.length ? `; ${gone.length} removed name(s) checked for stale references` : ""})`);
    return 0;
}

/** Lists every unresolved file reference in the tree's comments. */
function runDeadRefs(json) {
    const tree = trackedTree();
    const index = buildRefIndex(tree);
    const rows = [];
    for (const { file, info } of lexTree(tree)) {
        for (const { line, ref, text } of commentRefs(info)) {
            if (refIsFine(ref, info, index)) continue;
            rows.push({ file, line, ref, kind: refKind(ref, index), allowed: text.includes(ALLOW_TOKEN), text: text.trim() });
        }
    }
    if (json) {
        console.log(JSON.stringify(rows, null, 1));
        return 0;
    }
    // Opted-out lines are deliberate; count them, but list only the rest.
    const open = rows.filter((r) => !r.allowed);
    const by = {};
    for (const r of open) by[r.kind] = (by[r.kind] || 0) + 1;
    for (const r of open) console.log(`${r.file}:${r.line}: [${r.kind}] ${r.ref}`);
    const kinds = Object.entries(by).map(([k, v]) => `${k} ${v}`).join(", ");
    console.log(`\n${open.length} unresolved reference(s)${kinds ? `: ${kinds}` : ""}; ${rows.length - open.length} opted out with \`${ALLOW_TOKEN}\`.`);
    return 0;
}

function runCodeEqual(base) {
    const mb = mergeBaseWith(base);
    if (!mb) {
        console.error(`check-comment-hygiene: cannot resolve "${base}"`);
        return 2;
    }
    const rows = git("diff", "--name-status", "-M", mb, "--", "*.ts", "*.tsx", "*.rs")
        .split("\n")
        .filter(Boolean)
        .map((l) => l.split("\t"));
    let bad = 0;
    let compared = 0;
    for (const row of rows) {
        const status = row[0][0];
        const oldPath = row[1];
        const newPath = status === "R" || status === "C" ? row[2] : row[1];
        if (!isSourcePath(newPath) && !isSourcePath(oldPath)) continue;
        if (status === "A" || status === "D") {
            annotate("error", newPath, 1, `file ${status === "A" ? "added" : "deleted"}; a comment-only PR cannot add or delete source files.`);
            bad++;
            continue;
        }
        const before = gitShow(mb, oldPath);
        const after = readWorking(newPath);
        if (before === null || after === null) continue;
        compared++;
        const res = compareCodeEqual(before, after, langOf(newPath));
        for (const p of res.problems) {
            annotate("error", newPath, 1, p);
            bad++;
        }
        if (res.dropped.length) annotate("notice", newPath, 1, `citations dropped from comments (confirm each was narration or a duplicate): ${res.dropped.join(", ")}`);
    }
    // Only .ts/.tsx/.rs are compared. Anything else this branch touches (Cargo.toml,
    // .scss, scripts, package.json, docs) is listed so a "comment-only" PR that
    // also changes config is not mistaken for a verified one.
    const others = git("diff", "--name-only", "-M", mb)
        .split("\n")
        .filter((f) => f && !isSourcePath(f));
    if (others.length) {
        const shown = others.slice(0, 10).join(", ") + (others.length > 10 ? `, and ${others.length - 10} more` : "");
        annotate("warning", others[0], 1, `${others.length} changed file(s) are not compared (only .ts/.tsx/.rs are): ${shown}`);
    }
    if (bad) {
        console.error(`\ncheck-comment-hygiene --code-equal: ${bad} problem(s).`);
        return 1;
    }
    console.log(`check-comment-hygiene --code-equal: ok (${compared} source file(s) compared; code identical${others.length ? `; ${others.length} other changed file(s) not compared` : ""})`);
    return 0;
}

function runReport() {
    const files = git("ls-files", "--", "*.ts", "*.tsx", "*.rs").split("\n").filter(isSourcePath);
    const agg = {};
    const per = [];
    let markerLines = 0;
    for (const f of files) {
        const text = readWorking(f);
        if (text === null) continue;
        const info = lexSource(text, langOf(f));
        const s = statsOf(info);
        const key = `${langOf(f) === "rs" ? "Rust" : "TS"} ${isTestPath(f) ? "tests" : "non-test"}`;
        const a = (agg[key] ||= { files: 0, lines: 0, commentOnly: 0, commentChars: 0, chars: 0 });
        a.files++;
        a.lines += s.lines;
        a.commentOnly += s.commentOnly;
        a.commentChars += s.commentChars;
        a.chars += text.length;
        if (isTestPath(f)) continue;
        const markers = info.lines.filter((l) => isCommentOnly(l) && narrationRule(l.text)).length;
        markerLines += markers;
        per.push({ f, ...s, markers });
    }
    console.log("scope            files     lines  comment lines  ratio  comment chars share");
    for (const [k, a] of Object.entries(agg)) {
        console.log(`${k.padEnd(16)} ${String(a.files).padStart(5)} ${String(a.lines).padStart(9)} ${String(a.commentOnly).padStart(14)} ${((100 * a.commentOnly) / a.lines).toFixed(1).padStart(5)}% ${((100 * a.commentChars) / a.chars).toFixed(1).padStart(8)}%`);
    }
    console.log(`\nreview-history marker lines (non-test): ${markerLines}`);
    per.sort((x, y) => y.commentChars - x.commentChars);
    console.log("\ntop 20 non-test files by comment characters (tokens ~ chars / 4):");
    for (const p of per.slice(0, 20)) {
        console.log(`  ${p.f}  ${p.commentOnly}/${p.lines} lines (${(p.ratio * 100).toFixed(0)}%)  ~${Math.round(p.commentChars / 4)} tok  markers ${p.markers}`);
    }
    return 0;
}

function main(argv) {
    if (argv.includes("--help") || argv.includes("-h")) {
        console.log("usage: check-comment-hygiene.mjs [--report | --dead-refs [--json] | --code-equal <base>]");
        return 0;
    }
    if (argv.includes("--report")) return runReport();
    if (argv.includes("--dead-refs")) return runDeadRefs(argv.includes("--json"));
    const k = argv.indexOf("--code-equal");
    if (k !== -1) {
        if (!argv[k + 1]) {
            console.error("--code-equal needs a base ref, e.g. origin/main");
            return 2;
        }
        return runCodeEqual(argv[k + 1]);
    }
    return runGate();
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    process.exit(main(process.argv.slice(2)));
}
