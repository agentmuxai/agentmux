#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-transcript-scroll-writers.mjs: CI gate: the agent pane's transcript scrollTop is written in one place.
//
// THE RULE. Nothing under frontend/app/view/agent/ writes a scroller's position (`.scrollTop =`, `.scrollTop +=`,
// `.scrollTo(`, `.scrollBy(`, `.scrollIntoView(`) except scroll/one-way-flow.ts, which owns the follower and
// exports `scrollTranscriptTo` / `scrollTranscriptBy` for every caller that moves the transcript for its own
// reason (jump to a node, the older-history restore, keeping the rows on screen, a zoom anchor, the scroll
// hand-off). With a second writer the pane's "who moved it" question has several answers: that was most of the
// scroll bugs in docs/reports/REPORT_AGENT_PANE_CLEANUP_AUDIT_2026_10_30.md.
//
// The files in EXEMPT scroll something other than the transcript; each says what. Tests are not checked.
//
// Usage: node scripts/check-transcript-scroll-writers.mjs
// Exit 0 = clean, exit 1 = a write outside the writer was found.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, extname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const ROOT = "frontend/app/view/agent/";
const WRITER = "frontend/app/view/agent/scroll/one-way-flow.ts";

/** Files that write a scroller other than the transcript, and which one. */
export const EXEMPT = {
    "frontend/app/view/agent/scroll/preview-follow.ts": "a tool preview box",
    "frontend/app/view/agent/components/ToolOverlayLog.tsx": "a tool preview box",
    "frontend/app/view/agent/history/AgentHistoryView.tsx": "the History tab's own list",
    "frontend/app/view/agent/components/SlashCommandPicker.tsx": "the slash-command picker's list",
};

const WRITE = /\.scrollTop\s*[-+]?=(?!=)|\.scroll(?:To|By|IntoView)\s*\(/;

const isExempt = (path) =>
    path === WRITER || Object.hasOwn(EXEMPT, path) || /\.(?:test|bench)\.[cm]?[jt]sx?$/.test(path);

/** Writes in one file: `{ path, line, text }`, line 1-based, `path` repo-relative with forward slashes. */
export function checkFile(path, text) {
    if (!path.startsWith(ROOT) || isExempt(path)) return [];
    const out = [];
    text.split("\n").forEach((l, i) => {
        const t = l.trim();
        if (t.startsWith("//") || t.startsWith("*") || t.startsWith("/*")) return;
        if (WRITE.test(l)) out.push({ path, line: i + 1, text: t });
    });
    return out;
}

function walk(dir, out = []) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        if (entry.name === "node_modules" || entry.name === "dist") continue;
        const full = join(dir, entry.name);
        if (entry.isDirectory()) walk(full, out);
        else if ([".ts", ".tsx"].includes(extname(entry.name))) out.push(full);
    }
    return out;
}

/** Every write outside the writer under `root`/frontend/app/view/agent. */
export function collect(root = REPO_ROOT) {
    return walk(join(root, ROOT)).flatMap((full) =>
        checkFile(relative(root, full).split(sep).join("/"), readFileSync(full, "utf8"))
    );
}

if (isMain(import.meta.url)) {
    const found = collect();
    if (found.length === 0) {
        console.log("OK: the transcript's scrollTop is written only in scroll/one-way-flow.ts.");
    } else {
        console.error("ERROR: a scroll write outside the transcript's one writer. Move the transcript with");
        console.error(`scrollTranscriptTo / scrollTranscriptBy from ${WRITER}; if this scrolls another element,`);
        console.error("add the file to EXEMPT in scripts/check-transcript-scroll-writers.mjs with what it scrolls:");
        for (const v of found) console.error(`  ${v.path}:${v.line}: ${v.text}`);
        process.exit(1);
    }
}
