// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Whether a script module is the one `node` was asked to run, as opposed to
// being imported (by a test, or another script). Call it as
// `if (isMain(import.meta.url)) main();`.
//
// One copy, because the hand-written ones disagreed and one was wrong: on
// Windows `file://${argv[1]}` builds `file://C:/…` while import.meta.url is
// `file:///C:/…`, so check-new-spec-status.mjs silently never ran there. The
// comparison resolves symlinks on both sides and ignores case on Windows.
//
// docs/specs/PLAN_CODEX_GATE_TRANSIENT_FAILURES_AND_CI_SCRIPT_DRY_2026_10_14.md §3.6

import { realpathSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function isMain(metaUrl, argv1 = process.argv[1], platform = process.platform) {
    if (!argv1) return false;
    try {
        const self = realpathSync(fileURLToPath(metaUrl));
        const invoked = realpathSync(resolve(argv1));
        return platform === "win32" ? self.toLowerCase() === invoked.toLowerCase() : self === invoked;
    } catch {
        // Not a file: URL (some test runners), or a path that doesn't exist.
        return false;
    }
}
