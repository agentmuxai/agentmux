// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The lab's path options as absolute paths. On Windows a relative `--portable` (the lab ran
// `..\folder\agentmux.exe`) left the window host crashing at start (exit 0x80000003, "host_give_up"), with the
// same folder given as an absolute path starting fine; the lab then only said that no page had opened. The
// launcher works out the paths of its host and srv from the one it was started as, so the lab starts it from an
// absolute one whatever the shell's folder.

import { resolve } from "node:path";

/** The options that name a file or folder. */
export const PATH_OPTIONS = ["portable", "exe", "srv", "memtree", "out"];

/** `args` with every path option (the ones given) made absolute against `cwd`. */
export function absolutePaths(args, cwd = process.cwd()) {
    const out = { ...args };
    for (const key of PATH_OPTIONS) {
        if (typeof out[key] === "string" && out[key] !== "") out[key] = resolve(cwd, out[key]);
    }
    return out;
}
