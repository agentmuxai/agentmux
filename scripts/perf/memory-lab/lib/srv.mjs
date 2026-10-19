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

/**
 * Copies `portable` into `intoDir` and overwrites its srv with `srvPath`, keeping the
 * name the launcher looks for. Returns the copy's path and what was put in it.
 */
export function withReplacedSrv(portable, srvPath, intoDir) {
    if (!existsSync(srvPath) || !statSync(srvPath).isFile()) throw new Error(`--srv ${srvPath} is not a file`);
    const copy = join(intoDir, "portable-with-srv");
    rmSync(copy, { recursive: true, force: true });
    cpSync(portable, copy, { recursive: true });
    const files = readdirSync(copy, { recursive: true, withFileTypes: true }).filter((e) => e.isFile()).map((e) => join(e.parentPath ?? e.path, e.name).slice(copy.length + 1));
    const target = srvTarget(files);
    copyFileSync(srvPath, join(copy, target));
    const sha256 = createHash("sha256").update(readFileSync(srvPath)).digest("hex").slice(0, 16);
    return { portable: copy, replaced: { file: target, from: srvPath, bytes: statSync(srvPath).size, sha256 } };
}
