// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Measuring an srv change without building the whole app: a copy of a portable
// whose agentmux-srv is replaced by the one under test. The host, the renderer
// and the packaging stay the same, so only srv differs.

import { createHash } from "node:crypto";
import { copyFileSync, cpSync, existsSync, readFileSync, readdirSync, rmSync, statSync } from "node:fs";
import { basename, join } from "node:path";

/** The one file of a portable that is the srv: its name starts with `agentmux-srv`. Throws on none or several. */
export function srvTarget(relPaths) {
    const found = relPaths.filter((p) => /^agentmux-srv/i.test(basename(p)));
    if (found.length !== 1) {
        throw new Error(`expected one agentmux-srv in the portable, found ${found.length}${found.length ? `: ${found.join(", ")}` : ""}`);
    }
    return found[0];
}

/** The commit a portable's folder name says it was built from (`agentmux-0.59.32+gfe727c686...`), or null. */
export function portableCommit(portable) {
    return /\+g([0-9a-f]{7,40})/i.exec(basename(portable))?.[1]?.toLowerCase() ?? null;
}

/** Why a replacement srv may not match the portable's frontend, or null: both commits known and different. */
export function driftWarning(portable, srvCommit) {
    const built = portableCommit(portable);
    if (!built || !srvCommit) return null;
    const a = built.toLowerCase();
    const b = srvCommit.toLowerCase();
    if (a.startsWith(b) || b.startsWith(a)) return null;
    return `the portable was built from ${built} and the replacement srv from ${srvCommit}: the srv and the frontend may have drifted (a protocol-compatible srv can still leave the frontend unable to create agents). Build both from one commit, with only the change under test differing.`;
}

/**
 * Copies `portable` into `intoDir` and overwrites its srv with `srvPath`, keeping the
 * name the launcher looks for. Returns the copy's path and what was put in it.
 */
export function withReplacedSrv(portable, srvPath, intoDir) {
    if (!existsSync(srvPath) || !statSync(srvPath).isFile()) throw new Error(`--srv ${srvPath} is not a file`);
    // Under the source's own name: portableExe tells a macOS bundle by its ".app" and a folder by what is in it.
    const holder = join(intoDir, "portable-with-srv");
    const copy = join(holder, basename(portable));
    rmSync(holder, { recursive: true, force: true });
    // Symlinks stay as they are: rewritten to absolute paths they would lead back into the original.
    cpSync(portable, copy, { recursive: true, verbatimSymlinks: true });
    const files = readdirSync(copy, { recursive: true, withFileTypes: true }).filter((e) => e.isFile()).map((e) => join(e.parentPath ?? e.path, e.name).slice(copy.length + 1));
    const target = srvTarget(files);
    copyFileSync(srvPath, join(copy, target));
    const sha256 = createHash("sha256").update(readFileSync(srvPath)).digest("hex").slice(0, 16);
    return { portable: copy, replaced: { file: target, from: srvPath, bytes: statSync(srvPath).size, sha256, portableCommit: portableCommit(portable) } };
}
