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
    /** The pane's last compaction boundary (ms), or null: it moves forward when a compaction finishes. */
    boundaryAt: () => number | null;
    /** Whether the pane has a turn in flight. A compaction that failed, or that the person interrupted, ends its turn with no boundary. */
    turnBusy: () => boolean;
    /** Moves the agent to the model once the conversation is compacted. */
    switchTo: () => Promise<void>;
    /** A test passes its own clock. */
    wait?: (ms: number) => Promise<void>;
    now?: () => number;
    pollMs?: number;
    timeoutMs?: number;
    /** How many polls in a row the pane must be idle, after it was busy, before the compaction is called ended without a boundary. */
    idlePolls?: number;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** Resolves once the agent is switched; rejects with the reason when the compaction does not finish in time or the switch fails. */
export async function compactThenSwitch(deps: CompactThenSwitchDeps): Promise<void> {
    const { wait = sleep, now = Date.now, pollMs = 500, timeoutMs = COMPACT_WAIT_MS, idlePolls = 3 } = deps;
    const before = deps.boundaryAt();
    // The compaction is done when the boundary has moved forward from where it was when it was asked for.
    const compacted = () => {
        const at = deps.boundaryAt();
        return at != null && (before == null || at > before);
    };
    await deps.compact();
    const deadline = now() + timeoutMs;
    let sawTurn = false;
    let idle = 0;
    while (!compacted()) {
        if (now() >= deadline) throw new Error("The conversation was not compacted in time; the model was not changed.");
        // The turn that ran `/compact` has ended and no boundary came (it failed, or it was interrupted): stop
        // waiting, or a later compaction of any kind would switch the model after the operation was over.
        if (deps.turnBusy()) {
            sawTurn = true;
            idle = 0;
        } else if (sawTurn && ++idle >= idlePolls) {
            throw new Error("The compaction did not finish; the model was not changed.");
        }
        await wait(pollMs);
    }
    await deps.switchTo();
}
