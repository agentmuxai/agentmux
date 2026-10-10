// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The scratch folder verify-shortcuts.mjs gives the Files pane for the rows
// that change files on disk (--files-mutate), and the check that keeps those
// rows inside it. docs/specs/PLAN_SHORTCUT_KINKS_2026_10_10.md, L2 (K15): the
// first runs pointed the pane at a folder of the caller's choosing, and the
// Files rows created and trashed folders a few levels above it.
//
// The tree is three folders deep, so files:up (run twice, at L1 and L2) and
// back/forward stay inside it.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export const TEMP_PREFIX = "agentmux-verify-shortcuts-";

/**
 * True when `inner` is `outer` or below it. Pure: both paths are compared as
 * given, case-insensitively on Windows (and macOS's default volume), and a
 * sibling that merely starts with the same letters (`/tmp/a` vs `/tmp/ab`)
 * is outside.
 */
export function isWithinPath(inner, outer, platform = process.platform) {
    const p = platform === "win32" ? path.win32 : path.posix;
    const fold = platform === "win32" || platform === "darwin" ? (s) => s.toLowerCase() : (s) => s;
    const rel = p.relative(fold(p.resolve(outer)), fold(p.resolve(inner)));
    return rel === "" || (!rel.startsWith("..") && !p.isAbsolute(rel));
}

/** `p` with symlinks resolved when it exists (macOS's /var is /private/var). */
export function canonical(p) {
    try {
        return fs.realpathSync.native(p);
    } catch {
        return path.resolve(p);
    }
}

/** True when the folder a Files pane shows is inside the tree at `root`. */
export function insideTree(shown, root) {
    return Boolean(shown) && isWithinPath(canonical(shown), canonical(root));
}

/**
 * Makes a fresh tree under the OS temp folder: `<root>/l1/l2/start`, with two
 * text files, a Markdown file and a subfolder in `start` for the rows to act
 * on. Returns the root and the folder to point the pane at.
 */
export function makeTempTree() {
    const root = canonical(fs.mkdtempSync(path.join(os.tmpdir(), TEMP_PREFIX)));
    const start = path.join(root, "l1", "l2", "start");
    fs.mkdirSync(path.join(start, "sub"), { recursive: true });
    fs.writeFileSync(path.join(start, "notes.txt"), "line one\nline two\n");
    fs.writeFileSync(path.join(start, "b.txt"), "b\n");
    fs.writeFileSync(path.join(start, "notes.md"), "# Notes\n\nhello\n");
    fs.writeFileSync(path.join(start, "sub", "inner.txt"), "inner\n");
    return { root, start };
}

/** Removes a tree makeTempTree made; refuses any other folder. */
export function removeTempTree(root) {
    const tmp = canonical(os.tmpdir());
    const r = canonical(root);
    if (!path.basename(r).startsWith(TEMP_PREFIX) || path.dirname(r) !== tmp) {
        throw new Error(`refusing to remove ${root}: not a tree verify-shortcuts made`);
    }
    fs.rmSync(r, { recursive: true, force: true });
}
