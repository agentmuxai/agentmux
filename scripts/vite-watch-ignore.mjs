// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Which paths the Vite dev server's file watcher skips (server.watch.ignored in vite.config.ts).
//
// The watcher keeps one OS watch (a Windows handle) per file it takes in, and a dev server stays up
// for hours, so a tree the frontend never imports costs handles and memory for nothing: a `task dev`
// Vite held 153,000 file handles and 5.3 GB while watching an instance's data home. Everything the
// dev server serves lives under frontend/ (plus public/ and the root config files), so the other
// top-level directories are skipped by name. A glob list could not cover an alternative cargo target
// directory (`target-<agent>` held about 10,000 handles on one server), and it only matches when the
// root's spelling is the same as the watcher's; this function compares the root-relative path, with
// the case ignored on Windows.

import path from "node:path";

// Top-level directories of the repo that no page, module or asset of the dev server comes from.
export const UNWATCHED_TOP_DIRS = new Set([
    ".changesets",
    ".claude",
    ".dev-home",
    ".github",
    "crates",
    "dist",
    "docker",
    "docs",
    "packaging",
    "target",
]);

// Alternative cargo target directories (CARGO_TARGET_DIR=target-<name>) at the root.
const TARGET_PREFIX = "target-";

const fold = (s) => (process.platform === "win32" ? s.toLowerCase() : s);

/**
 * @param {string} root the repo root the dev server runs in
 * @returns {(p: string) => boolean} true for a path the watcher must not take in
 */
export function makeWatchIgnore(root) {
    const rootNorm = fold(path.resolve(root).replace(/\\/g, "/")).replace(/\/+$/, "");
    return (p) => {
        const norm = fold(String(p).replace(/\\/g, "/"));
        if (!norm.startsWith(rootNorm + "/")) return false;
        const top = norm.slice(rootNorm.length + 1).split("/", 1)[0];
        return UNWATCHED_TOP_DIRS.has(top) || top.startsWith(TARGET_PREFIX);
    };
}
