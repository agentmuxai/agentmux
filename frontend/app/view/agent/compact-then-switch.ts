// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Compact, then switch": a model that refused the conversation, or is too small for it, may take it once it is
 * compacted. Runs `/compact` in the pane, waits for the compaction boundary the harness reports, then moves the agent.
 */

/** How long to wait for the compaction to report before giving up and leaving the agent where it is. */
export const COMPACT_WAIT_MS = 10 * 60 * 1000;

export interface CompactThenSwitchDeps {
    /** Starts `/compact` in the pane. */
    compact: () => Promise<void> | void;
    /** The pane's last compaction boundary (ms), or null: it changes when a compaction finishes. */
    boundaryAt: () => number | null;
    /** Moves the agent to the model once the conversation is compacted. */
    switchTo: () => Promise<void>;
    /** A test passes its own clock. */
    wait?: (ms: number) => Promise<void>;
    now?: () => number;
    pollMs?: number;
    timeoutMs?: number;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** Resolves once the agent is switched; rejects with the reason when the compaction does not finish in time or the switch fails. */
export async function compactThenSwitch(deps: CompactThenSwitchDeps): Promise<void> {
    const { wait = sleep, now = Date.now, pollMs = 500, timeoutMs = COMPACT_WAIT_MS } = deps;
    const before = deps.boundaryAt();
    await deps.compact();
    const deadline = now() + timeoutMs;
    while (deps.boundaryAt() === before) {
        if (now() >= deadline) throw new Error("The conversation was not compacted in time; the model was not changed.");
        await wait(pollMs);
    }
    await deps.switchTo();
}
