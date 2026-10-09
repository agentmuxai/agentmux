// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How far a test run has got, read from a command's live output: the live
 * status says "Running the tests · 41/123" instead of only the time. Known
 * runners only (cargo test, pytest, vitest, jest); anything else reads as no
 * progress, never a guess.
 *
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.9.
 */

/** Feed one output chunk; returns the progress line when it changed. */
export type TestProgressTracker = (chunk: string) => string | null;

export function createTestProgressTracker(): TestProgressTracker {
    let total: number | null = null;
    let done = 0;
    let failed = 0;
    let last: string | null = null;
    const changed = (next: string | null) => {
        if (next == null || next === last) return null;
        last = next;
        return next;
    };
    return (chunk) => {
        let next: string | null = null;
        for (const raw of chunk.split(/\r?\n/)) {
            const line = raw.replace(/\x1b\[[0-9;]*m/g, "").trimEnd();
            let m: RegExpExecArray | null;
            // cargo test: "running 123 tests", then one line per test.
            if ((m = /^running (\d+) tests?$/.exec(line))) {
                total = Number(m[1]);
                done = 0;
                failed = 0;
                continue;
            }
            if (total != null && /^test .+ \.\.\. (ok|FAILED|ignored)/.test(line)) {
                done += 1;
                if (line.endsWith("FAILED")) failed += 1;
                next = `${done}/${total}${failed ? `, ${failed} failed` : ""}`;
                continue;
            }
            // Final summaries.
            if ((m = /^test result: \w+\. (\d+) passed; (\d+) failed/.exec(line))) {
                next = Number(m[2]) > 0 ? `${m[1]} passed, ${m[2]} failed` : `${m[1]} passed`;
                total = null;
                continue;
            }
            if ((m = /^\s*Tests\s+(?:(\d+) failed \| )?(\d+) passed/.exec(line))) {
                next = m[1] ? `${m[2]} passed, ${m[1]} failed` : `${m[2]} passed`;
                continue;
            }
            if ((m = /^Tests:\s+(.+?),\s*\d+ total/.exec(line))) {
                next = m[1];
                continue;
            }
            if ((m = /^=+ (.+?) in [\d.]+s(?: \([^)]*\))? =+$/.exec(line))) {
                next = m[1];
                continue;
            }
            // pytest progress: "tests/test_x.py ....F..   [ 45%]".
            if ((m = /\[\s*(\d{1,3})%\]$/.exec(line))) next = `${m[1]}%`;
        }
        return changed(next);
    };
}
