// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How long srv's transcript store kept everyone waiting during a run. The store has one write
// connection behind a lock that every store operation waits for (crates/srv/src/backend/storage/
// filestore/core.rs); srv logs a warning when it was held 500 ms or more ("connection held a long
// time", held_ms) and when a caller waited that long for it ("slow to get the connection",
// waited_ms). The longest of those is a direct measure of the lock: a change that makes it worse
// shows here even when the run's memory and CPU are unchanged (issue #5141).

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

/** The longest hold and wait in the text of an srv log (JSON lines); 0 when there were none. */
export function parseStoreLock(text) {
    const out = { longestHeldMs: 0, longestHeldSite: null, holds: 0, longestWaitedMs: 0, waits: 0 };
    for (const line of text.split("\n")) {
        if (!line.includes("filestore: ")) continue;
        let rec;
        try {
            rec = JSON.parse(line);
        } catch {
            continue;
        }
        const f = rec.fields ?? {};
        if (f.message === "filestore: connection held a long time" && Number.isFinite(f.held_ms)) {
            out.holds++;
            if (f.held_ms > out.longestHeldMs) {
                out.longestHeldMs = f.held_ms;
                out.longestHeldSite = String(f.site ?? "").replace(/\\/g, "/").replace(/^crates\/srv\/src\//, "");
            }
        } else if (f.message === "filestore: slow to get the connection" && Number.isFinite(f.waited_ms)) {
            out.waits++;
            out.longestWaitedMs = Math.max(out.longestWaitedMs, f.waited_ms);
        }
    }
    return out;
}

/** Folds several logs' stats (srv rotates daily and restarts) into one. */
export function mergeStoreLock(list) {
    const out = { longestHeldMs: 0, longestHeldSite: null, holds: 0, longestWaitedMs: 0, waits: 0 };
    for (const s of list) {
        out.holds += s.holds;
        out.waits += s.waits;
        out.longestWaitedMs = Math.max(out.longestWaitedMs, s.longestWaitedMs);
        if (s.longestHeldMs > out.longestHeldMs) {
            out.longestHeldMs = s.longestHeldMs;
            out.longestHeldSite = s.longestHeldSite;
        }
    }
    return out;
}

/** The store-lock stats of every srv log under a lab data folder, or null when there is no log to read. */
export function storeLockOf(home) {
    let files;
    try {
        files = readdirSync(home, { recursive: true, withFileTypes: true }).filter((e) => e.isFile() && /^agentmuxsrv.*\.log/.test(e.name));
    } catch {
        return null;
    }
    if (!files.length) return null;
    return mergeStoreLock(files.map((e) => parseStoreLock(readFileSync(join(e.parentPath ?? e.path, e.name), "utf8"))));
}
