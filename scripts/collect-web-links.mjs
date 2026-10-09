#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// collect-web-links.mjs — the input for the weekly web-link report
// (.github/workflows/weekly-web-links.yml), plan step 20 of the CI test-speed
// and DRY follow-ups plan (PR #4534).
//
//   node scripts/collect-web-links.mjs <out-dir>
//
// Collects the web URLs in tracked `docs/**/*.md` and in code comments (the
// files check-comment-hygiene.mjs reads, through its own lexer, so a URL in a
// string literal is not a comment and is left alone), keeps only the hosts we
// check, and writes one `<out-dir>/<repo path>.txt` per source file, each URL
// on the same line number as in the source (other lines left blank). lychee
// then checks `<out-dir>/**/*.txt`, so its "(at LINE:COL)" points at the
// source line, and its report names the file by that mirrored path.
//
// Doing the filtering here rather than with lychee's include/exclude keeps the
// rules plain: lychee's `include` overrides every `exclude`, so "only these
// hosts, but not templated URLs" cannot be said in its regexes alone.
//
// Checked hosts: github.com, and our own web pages (agentmux.ai,
// www.agentmux.ai, docs.agentmux.ai). Other *.agentmux.ai names in the tree
// are service endpoints (OAuth, APIs, download paths with a version
// placeholder), not pages, so a GET on them says nothing about a link.
// Everything else is someone else's server, deliberately out of scope
// (plan step 20: CI must not depend on other people's servers).
//
// Skipped, as lychee does by default for Markdown: fenced code blocks and
// inline code spans in docs, which hold examples rather than pointers. Also
// skipped anywhere: URLs with a placeholder (`{`, `$`, `<`, `*`, `...`,
// OWNER/REPO-style capitals).
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { isScanPath, langOf, lexSource } from "./check-comment-hygiene.mjs";

const out = process.argv[2];
if (!out) {
    console.error("usage: collect-web-links.mjs <out-dir>");
    process.exit(2);
}

const HOST_RE = /^(?:github\.com|(?:www\.|docs\.)?agentmux\.ai)$/i;
const URL_RE = /https?:\/\/[^\s<>()[\]{}"'`|\\]+/g;
const PLACEHOLDER_RE = /[{}$<>*]|\.\.\.|\/(?:OWNER|REPO|ORG|USER|NUMBER|N)(?:\/|$)/;

/** The URL with trailing punctuation from the surrounding prose removed. */
function trimUrl(u) {
    return u.replace(/[.,;:!?'"*_~]+$/, "");
}

/** Matching URLs in one line of text. */
function urlsIn(text) {
    const found = [];
    for (const m of text.matchAll(URL_RE)) {
        const url = trimUrl(m[0]);
        if (PLACEHOLDER_RE.test(url.replace(/^https?:\/\//, ""))) continue;
        let host;
        try {
            host = new URL(url).hostname;
        } catch {
            continue;
        }
        if (HOST_RE.test(host)) found.push(url);
    }
    return found;
}

/** `[{ line, url }]` for a Markdown file, outside code fences and code spans. */
function markdownUrls(src) {
    const rows = [];
    let fence = null;
    src.split("\n").forEach((raw, idx) => {
        const fenceMatch = /^\s*(`{3,}|~{3,})/.exec(raw);
        if (fenceMatch) {
            const mark = fenceMatch[1][0];
            if (!fence) fence = mark;
            else if (fence === mark) fence = null;
            return;
        }
        if (fence) return;
        const text = raw.replace(/(`+)[^`]*?\1/g, " ");
        for (const url of urlsIn(text)) rows.push({ line: idx + 1, url });
    });
    return rows;
}

/** `[{ line, url }]` for a source file, from its comments only. */
function commentUrls(src, file) {
    const rows = [];
    lexSource(src, langOf(file)).lines.forEach((l, idx) => {
        if (l.text) for (const url of urlsIn(l.text)) rows.push({ line: idx + 1, url });
    });
    return rows;
}

const tracked = execFileSync("git", ["ls-files"], { encoding: "utf8" }).split("\n").filter(Boolean);
let files = 0;
let urls = 0;
const unique = new Set();
for (const file of tracked) {
    const isDoc = file.startsWith("docs/") && file.endsWith(".md");
    if (!isDoc && (file.startsWith("docs/") || !isScanPath(file))) continue;
    let src;
    try {
        src = readFileSync(file, "utf8");
    } catch {
        continue;
    }
    const rows = isDoc ? markdownUrls(src) : commentUrls(src, file);
    if (!rows.length) continue;
    const target = join(out, `${file}.txt`);
    mkdirSync(dirname(target), { recursive: true });
    const lines = new Array(rows[rows.length - 1].line).fill("");
    for (const r of rows) lines[r.line - 1] += (lines[r.line - 1] ? " " : "") + r.url;
    writeFileSync(target, lines.join("\n") + "\n");
    files++;
    urls += rows.length;
    for (const r of rows) unique.add(r.url);
}
console.log(`collect-web-links: ${urls} URL(s), ${unique.size} unique, in ${files} file(s) -> ${out}/`);
