#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window modals stay few: a dialog about one pane or one agent covers only that pane
// (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md §2, §5). Without this check the count grows
// back one convenient `openModal(` at a time. It reads frontend/ and fails on:
//   - `openModal(` (a window modal) from a file that is not in KEEP, or more often than KEEP says;
//   - `<ConfirmModal` (its default scope is the window) the same way;
//   - `scope="window"` / `scope: "window"` the same way;
//   - `window.confirm`, `window.alert`, `window.prompt`, which freeze the renderer.
// A new dialog about one pane uses `openPaneModal(blockId, ...)`. A new window modal needs a line in KEEP,
// with the reason it concerns the whole app.
//
// Usage:  node scripts/check-window-modals.mjs      (exit 1 on a violation)

import { readdirSync, readFileSync } from "node:fs";
import { dirname, extname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

/** The primitives themselves: where a scope is chosen and a modal opened. */
export const PRIMITIVES = new Set([
    "frontend/app/element/modal.tsx",
    "frontend/app/element/confirm-modal.tsx",
    "frontend/app/store/modalmodel.ts",
]);

/**
 * Window modals that stay, by file: how many of each kind, and why. A kind is `openModal`, `confirmModal`
 * (`<ConfirmModal` with no `scope=`), `windowScope` (an explicit window scope).
 */
export const KEEP = new Map([
    ["frontend/app/window/window-close.ts", { openModal: 2, why: "closing or removing a window: the whole window" }],
    ["frontend/app/window/workspaces.ts", { openModal: 7, why: "switch, create, rename, pick, delete and import a workspace: the whole window" }],
    ["frontend/app/tab/tab-close-confirm-modal.tsx", { windowScope: 1, why: "closing a tab is asked from the tab bar, outside any pane's lock; it ends agents in every pane of the tab" }],
    ["frontend/app/block/upgrade-requests.tsx", { openModal: 1, confirmModal: 1, why: "an app upgrade restarts every agent" }],
    ["frontend/app/block/desktop-requests.tsx", { openModal: 1, why: "a desktop-control request that names no pane; one from an agent is a card in its pane" }],
    ["frontend/app/block/widget-install-requests.tsx", { openModal: 1, confirmModal: 1, why: "an install request with no agent pane (Settings, the CLI) has no pane to ask in; one from an agent is a card" }],
    ["frontend/app/store/open-command-palette.ts", { openModal: 1, why: "the command palette is an app-wide launcher" }],
    ["frontend/app/window/logo-menu.tsx", { openModal: 1, why: "the command palette entry of the logo menu" }],
    ["frontend/app/statusbar/ConfigStatus.tsx", { openModal: 1, why: "the config error: the status bar is the window's" }],
    ["frontend/app/store/ui-event-subs.ts", { openModal: 1, why: "srv's user-input request carries no pane or agent" }],
    ["frontend/app/statusbar/TokenBreakdownPopover.tsx", { confirmModal: 1, why: "reset the token counter: the status bar is the window's" }],
    ["frontend/app/window/window-close-confirm.tsx", { confirmModal: 2, why: "the confirmations of closing or removing a window: the whole window" }],
    ["frontend/app/window/workspace-modals.tsx", { confirmModal: 2, why: "the workspace confirmations: the whole window" }],
]);

const SCRIPT_EXT = new Set([".ts", ".tsx", ".js", ".jsx"]);
const isTest = (path) => /\.test\.[cm]?[jt]sx?$/.test(path) || path.includes("/__tests__/");
const lineAt = (text, index) => text.slice(0, index).split("\n").length;

const NEWLINE = String.fromCharCode(10);
const blank = (s) => s.replace(/[^\n]/g, " ");

/**
 * `text` with comments blanked out (newlines kept, so line numbers hold). Strings and template literals are
 * skipped over, so a `//` or `/*` inside one starts no comment. (A regex literal holding a quote can still
 * fool it; none in frontend/ does.)
 */
export function stripComments(text) {
    let out = "";
    let i = 0;
    while (i < text.length) {
        const c = text[i];
        const next = text[i + 1];
        if (c === "/" && next === "/") {
            const end = text.indexOf("\n", i);
            const stop = end === -1 ? text.length : end;
            out += blank(text.slice(i, stop));
            i = stop;
        } else if (c === "/" && next === "*") {
            const end = text.indexOf("*/", i + 2);
            const stop = end === -1 ? text.length : end + 2;
            out += blank(text.slice(i, stop));
            i = stop;
        } else if (c === '"' || c === "'" || c === "`") {
            // A quoted string ends on its line (an apostrophe in JSX text is not one); a template may run on.
            let j = i + 1;
            while (j < text.length && text[j] !== c && (c === "`" || text[j] !== NEWLINE)) j += text[j] === "\\" ? 2 : 1;
            out += text.slice(i, j + 1);
            i = j + 1;
        } else {
            out += c;
            i++;
        }
    }
    return out;
}

/** Where the JSX tag that opens at `start` (the `<`) ends: its first `>` outside braces and quotes. */
function tagEnd(code, start) {
    let depth = 0;
    for (let i = start + 1; i < code.length; i++) {
        const c = code[i];
        if (c === '"' || c === "'" || c === "`") {
            i++;
            while (i < code.length && code[i] !== c && (c === "`" || code[i] !== NEWLINE)) i += code[i] === "\\" ? 2 : 1;
        } else if (c === "{") depth++;
        else if (c === "}") depth--;
        else if (c === ">" && depth === 0) return i;
    }
    return code.length;
}

/** The `<ConfirmModal` tags in `code` that name no `scope`, as `{ index }`. */
function unscopedConfirmModals(code) {
    const out = [];
    for (const m of code.matchAll(/<ConfirmModal\b/g)) {
        const attrs = code.slice(m.index, tagEnd(code, m.index));
        // Top level only: blank what sits inside braces, so `onConfirm={() => scope="x"}` does not count.
        let depth = 0;
        const top = [...attrs].map((c) => (c === "{" ? (depth++, " ") : c === "}" ? (depth--, " ") : depth > 0 ? " " : c)).join("");
        if (!/\bscope\s*=/.test(top)) out.push({ index: m.index });
    }
    return out;
}

const KINDS = [
    ["openModal", /(?<![.\w])openModal\s*\(|\bmodalsModel\.openModal\s*\(/g],
    ["windowScope", /\bscope\s*[=:]\s*\{?\s*["']window["']/g],
];
const FORBIDDEN = [/\bwindow\.(?:confirm|alert|prompt)\s*\(/g];

/** `{ counts: { kind: n }, hits: [{ kind, line }] }` for one file's text. */
export function scan(text) {
    const code = stripComments(text);
    const counts = {};
    const hits = [];
    for (const [kind, re] of KINDS) {
        for (const m of code.matchAll(re)) {
            counts[kind] = (counts[kind] ?? 0) + 1;
            hits.push({ kind, line: lineAt(code, m.index) });
        }
    }
    for (const { index } of unscopedConfirmModals(code)) {
        counts.confirmModal = (counts.confirmModal ?? 0) + 1;
        hits.push({ kind: "confirmModal", line: lineAt(code, index) });
    }
    for (const re of FORBIDDEN) {
        for (const m of code.matchAll(re)) hits.push({ kind: "freezes-renderer", line: lineAt(code, m.index) });
    }
    hits.sort((a, b) => a.line - b.line);
    return { counts, hits };
}

/** Violations in one file: `{ path, line, rule }`. `path` is repo-relative with forward slashes. */
export function checkFile(path, text) {
    if (!SCRIPT_EXT.has(extname(path)) || isTest(path) || PRIMITIVES.has(path)) return [];
    const { counts, hits } = scan(text);
    const keep = KEEP.get(path) ?? {};
    const out = [];
    for (const h of hits) {
        if (h.kind === "freezes-renderer") out.push({ path, line: h.line, rule: h.kind });
    }
    for (const kind of ["openModal", "confirmModal", "windowScope"]) {
        const n = counts[kind] ?? 0;
        const allowed = keep[kind] ?? 0;
        if (n <= allowed) continue;
        for (const h of hits.filter((x) => x.kind === kind).slice(allowed)) out.push({ path, line: h.line, rule: kind, found: n, allowed });
    }
    return out;
}

function walk(dir, out = []) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        if (entry.name === "node_modules" || entry.name === "dist") continue;
        const full = join(dir, entry.name);
        if (entry.isDirectory()) walk(full, out);
        else out.push(full);
    }
    return out;
}

/** Every violation under `root`/frontend, and the KEEP entries that no longer hold anything. */
export function collect(root = REPO_ROOT) {
    const violations = [];
    const seen = new Map();
    for (const full of walk(join(root, "frontend"))) {
        const path = relative(root, full).split(sep).join("/");
        if (!SCRIPT_EXT.has(extname(path)) || isTest(path)) continue;
        const text = readFileSync(full, "utf8");
        violations.push(...checkFile(path, text));
        seen.set(path, scan(text).counts);
    }
    const stale = [];
    for (const [path, keep] of KEEP) {
        const counts = seen.get(path) ?? {};
        for (const kind of ["openModal", "confirmModal", "windowScope"]) {
            if ((keep[kind] ?? 0) > (counts[kind] ?? 0)) stale.push({ path, kind, allowed: keep[kind], found: counts[kind] ?? 0 });
        }
    }
    return { violations, stale };
}

const ADVICE = {
    openModal: "a window modal: about one pane or agent, use openPaneModal(blockId, ...); about the whole app, add it to KEEP with the reason",
    confirmModal: "<ConfirmModal> defaults to the window: give it scope=\"pane\" (or open it with openPaneModal), or add it to KEEP with the reason",
    windowScope: "an explicit window scope: add it to KEEP with the reason it concerns the whole app",
    "freezes-renderer": "window.confirm/alert/prompt freeze the renderer: use createConfirm() or a pane modal",
};

function main() {
    const { violations, stale } = collect();
    for (const v of violations) {
        const count = v.found ? ` (${v.found} found, ${v.allowed} allowed)` : "";
        console.error(`${v.path}:${v.line}: ${v.rule}${count} - ${ADVICE[v.rule]}`);
    }
    for (const s of stale) {
        console.error(`scripts/check-window-modals.mjs: KEEP lists ${s.allowed} ${s.kind} in ${s.path}, found ${s.found}: lower or remove the entry`);
    }
    if (violations.length || stale.length) process.exit(1);
    console.log(`OK: every window modal is in KEEP (${KEEP.size} files); the rest of the dialogs are pane modals.`);
}

if (isMain(import.meta.url)) main();
