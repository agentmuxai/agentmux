#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-utf8.mjs: every tracked text source file is strict UTF-8 with no byte-order mark.
//
// The production frontend build (rolldown, via vite) stops on a file that is not valid UTF-8 ("stream did not
// contain valid UTF-8"), but tsc and vitest read past such a byte, so a Windows-1252 character (a section sign, a
// euro sign) saved by a tool that wrote the wrong encoding passes every PR check and breaks the packaged build.
// No PR job runs that build. This reads the bytes itself, in about a second, and names each bad byte.
//
//   node scripts/check-utf8.mjs      checks `git ls-files`; prints file, byte offset, line and what was found
//
// A file with a NUL byte is binary and skipped. Exit 1 on any problem, 0 otherwise.

import { spawnSync } from "node:child_process";
import { lstatSync, readFileSync } from "node:fs";
import { dirname, extname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const EXTENSIONS = new Set([".ts", ".tsx", ".js", ".mjs", ".cjs", ".rs", ".scss", ".css", ".json", ".md", ".yml", ".yaml", ".toml", ".sh", ".ps1", ".html"]);
// Problems listed per file; one broken save usually leaves several.
const MAX_PER_FILE = 5;

const hex = (b) => `0x${b.toString(16).toUpperCase().padStart(2, "0")}`;

/** Length of the well-formed UTF-8 sequence starting at `i`, or 0 when the bytes there are not one (overlong, surrogate, past U+10FFFF, truncated). */
function sequenceLength(buf, i) {
    const b = buf[i];
    if (b < 0x80) return 1;
    const cont = (k, lo = 0x80, hi = 0xbf) => i + k < buf.length && buf[i + k] >= lo && buf[i + k] <= hi;
    if (b >= 0xc2 && b <= 0xdf) return cont(1) ? 2 : 0;
    if (b >= 0xe0 && b <= 0xef) {
        const lo = b === 0xe0 ? 0xa0 : 0x80;
        const hi = b === 0xed ? 0x9f : 0xbf;
        return cont(1, lo, hi) && cont(2) ? 3 : 0;
    }
    if (b >= 0xf0 && b <= 0xf4) {
        const lo = b === 0xf0 ? 0x90 : 0x80;
        const hi = b === 0xf4 ? 0x8f : 0xbf;
        return cont(1, lo, hi) && cont(2) && cont(3) ? 4 : 0;
    }
    return 0;
}

/** The problems in one file's bytes: [{ offset, line, what }], at most MAX_PER_FILE, or [] for a binary file. */
export function utf8Problems(buf) {
    if (buf.includes(0)) return [];
    const out = [];
    let line = 1;
    let i = 0;
    if (buf.length >= 3 && buf[0] === 0xef && buf[1] === 0xbb && buf[2] === 0xbf) {
        out.push({ offset: 0, line: 1, what: "byte-order mark (EF BB BF)" });
        i = 3;
    }
    while (i < buf.length && out.length < MAX_PER_FILE) {
        const n = sequenceLength(buf, i);
        if (n === 0) {
            out.push({ offset: i, line, what: `invalid UTF-8 byte ${hex(buf[i])}` });
            i += 1;
            continue;
        }
        if (buf[i] === 0x0a) line += 1;
        i += n;
    }
    return out;
}

/** Tracked files with a checked extension, relative to `root`. */
export function trackedSources(root) {
    const r = spawnSync("git", ["ls-files", "-z"], { cwd: root, encoding: "buffer", maxBuffer: 256 * 1024 * 1024 });
    if (r.status !== 0) throw new Error(`git ls-files failed: ${r.stderr}`);
    return r.stdout
        .toString("utf8")
        .split("\0")
        .filter((p) => p && EXTENSIONS.has(extname(p).toLowerCase()));
}

function main() {
    let files = 0;
    let bad = 0;
    for (const rel of trackedSources(ROOT)) {
        const abs = join(ROOT, rel);
        let buf;
        try {
            // A submodule, a symlink or a file removed from the working tree is not source to read.
            if (!lstatSync(abs).isFile()) continue;
            buf = readFileSync(abs);
        } catch {
            continue;
        }
        files += 1;
        for (const p of utf8Problems(buf)) {
            bad += 1;
            console.error(`${rel}: byte offset ${p.offset}, line ${p.line}: ${p.what}`);
        }
    }
    if (bad) {
        console.error(`\ncheck-utf8: ${bad} problem(s). Save the file as UTF-8 without a byte-order mark; an invalid byte is usually a Windows-1252 character (0xA7 is the section sign) from an editor or tool that wrote the wrong encoding.`);
        return 1;
    }
    console.log(`check-utf8: ok (${files} files)`);
    return 0;
}

if (isMain(import.meta.url)) process.exit(main());
