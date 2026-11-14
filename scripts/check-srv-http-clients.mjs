#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-srv-http-clients.mjs — CI gate: srv, the MCP server, bashwrap and the desktop host build their HTTP clients in one
// place each.
//
// THE RULE. Every HTTP client in crates/srv is built through crates/srv/src/http_client.rs (`builder()` and `client()`); every
// one in crates/mcp, crates/bashwrap and crates/cef (which only talk to the local server) through
// crates/common/src/local_http.rs. reqwest 0.13 reads the machine's certificates when it builds a client and refuses to build
// one on a Linux machine that has none, and `reqwest::Client::new()` turns that refusal into a panic: the server and the MCP
// server died at start on a machine without `ca-certificates`. The helpers build a client that cannot fail for that reason
// (srv's trusts nothing there: every https request fails and the log and the connectivity error say why; the local-only one
// never reads certificates), so no site needs to handle it. A new direct construction brings the panic back.
//
// Fails on `reqwest::Client::new()`, `reqwest::Client::builder()`, `reqwest::Client::default()`, `reqwest::ClientBuilder::new()`
// and `reqwest::get(` (which builds a default client), and on `Client::new()` or `Client::builder()` in a file that imports
// `reqwest::Client`, in the source of those four crates, outside comments and test code (a test file or folder by its name,
// and any item marked `#[cfg(test)]`, a test module included).
//
// Usage: node scripts/check-srv-http-clients.mjs
// Exit 0 = clean, exit 1 = a direct construction was found.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, extname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const CRATES = ["crates/srv/src", "crates/mcp/src", "crates/bashwrap/src", "crates/cef/src"];
const HELPER = "crates/srv/src/http_client.rs";
const FULL = /reqwest::(?:Client::(?:new|builder|default)\b|ClientBuilder::new\b|get\s*\()/;
const SHORT = /(?<![\w:])Client::(?:new|builder|default)\s*\(/;
const IMPORTS_CLIENT = /use\s+reqwest::(?:\{[^}]*\bClient\b[^}]*\}|Client\b)/;

const isTestFile = (path) => /(^|\/)(tests|test_support|test_stub|tls_test_init)(\.rs|\/)/.test(path) || /(^|\/)[^/]*_tests?\.rs$/.test(path) || path.endsWith("/tests.rs");

/** `text` with comments, string and character literals blanked out (newlines kept), so braces and attributes can be counted. */
export function blank(text) {
    let out = "";
    let i = 0;
    const n = text.length;
    const keep = (c) => (c === "\n" ? "\n" : " ");
    while (i < n) {
        const c = text[i];
        const d = text[i + 1];
        if (c === "/" && d === "/") {
            while (i < n && text[i] !== "\n") (out += " "), i++;
        } else if (c === "/" && d === "*") {
            let depth = 0;
            while (i < n) {
                if (text[i] === "/" && text[i + 1] === "*") (depth++, (out += "  "), (i += 2));
                else if (text[i] === "*" && text[i + 1] === "/") (depth--, (out += "  "), (i += 2), depth === 0 && (i = i));
                else (out += keep(text[i]), i++);
                if (depth === 0) break;
            }
        } else if (c === "r" && /^r#*"/.test(text.slice(i, i + 12)) && !/[\w]/.test(text[i - 1] ?? "")) {
            const hashes = /^r(#*)"/.exec(text.slice(i, i + 12))[1];
            const close = '"' + hashes;
            let j = i + 2 + hashes.length;
            const end = text.indexOf(close, j);
            const stop = end < 0 ? n : end + close.length;
            for (; i < stop; i++) out += keep(text[i]);
        } else if (c === '"') {
            out += " ";
            i++;
            while (i < n && text[i] !== '"') {
                if (text[i] === "\\") (out += " ", i++);
                if (i < n) (out += keep(text[i]), i++);
            }
            if (i < n) (out += " ", i++);
        } else if (c === "'" && /^'(\\.[^']*|[^\\'])'/.test(text.slice(i, i + 12))) {
            const m = /^'(\\.[^']*|[^\\'])'/.exec(text.slice(i, i + 12))[0];
            out += " ".repeat(m.length);
            i += m.length;
        } else {
            out += c;
            i++;
        }
    }
    return out;
}

/** The 0-based lines that are test code in a file: items marked `#[cfg(test)]` (a module, a function, a use) and, with an inner `#![cfg(test)]`, all of it. */
export function testLines(text) {
    const b = blank(text);
    const lines = b.split("\n");
    const isTest = new Array(lines.length).fill(false);
    if (/^\s*#!\[cfg\(test\)\]/m.test(b)) return isTest.fill(true);
    for (let i = 0; i < lines.length; i++) {
        if (!/^\s*#\[cfg\(test\)\]\s*$/.test(lines[i])) continue;
        let depth = 0;
        let started = false;
        let end = i;
        scan: for (let j = i + 1; j < lines.length; j++) {
            for (const ch of lines[j]) {
                if (ch === "{") (depth++, (started = true));
                else if (ch === "}") depth--;
                else if (ch === ";" && !started && depth === 0) (end = j, (j = lines.length));
                if (started && depth === 0) (end = j, (j = lines.length));
            }
            if (j >= lines.length) break scan;
            end = j;
        }
        for (let k = i; k <= end; k++) isTest[k] = true;
        i = end;
    }
    return isTest;
}

/** Direct constructions in one file: `{ path, line, text }`, line 1-based, `path` repo-relative with forward slashes. */
export function checkFile(path, text) {
    if (path === HELPER || isTestFile(path)) return [];
    const code = blank(text).split("\n");
    const test = testLines(text);
    const imports = IMPORTS_CLIENT.test(blank(text));
    const raw = text.split("\n");
    const out = [];
    for (const [i, l] of code.entries()) {
        if (test[i]) continue;
        if (FULL.test(l) || (imports && SHORT.test(l))) out.push({ path, line: i + 1, text: raw[i].trim() });
    }
    return out;
}

function walk(dir, out = []) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const full = join(dir, entry.name);
        if (entry.isDirectory()) walk(full, out);
        else if (extname(entry.name) === ".rs") out.push(full);
    }
    return out;
}

/** Every direct construction under `root`, in the source of the four crates. */
export function collect(root = REPO_ROOT) {
    return CRATES.flatMap((c) => walk(join(root, c))).flatMap((full) => checkFile(relative(root, full).split(sep).join("/"), readFileSync(full, "utf8")));
}

if (isMain(import.meta.url)) {
    const found = collect();
    if (found.length === 0) {
        console.log("OK: srv, the MCP server, bashwrap and the desktop host build their HTTP clients only through their helpers.");
    } else {
        console.error("ERROR: a reqwest client built directly (in srv use crate::http_client::builder() or ::client(); in mcp, bashwrap and");
        console.error(`cef use agentmux_common::local_http; both still build a client on a machine with no system certificates; see ${HELPER}):`);
        for (const v of found) console.error(`  ${v.path}:${v.line}: ${v.text}`);
        process.exit(1);
    }
}
