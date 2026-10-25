// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** `retired` with `rowKey` retired at `lastEventAt`: the same Map when it already is, so a
 *  repeated retire notifies nobody and rebuilds nothing. */
export function withRetiredEntry(retired: Map<string, number>, rowKey: string, lastEventAt: number): Map<string, number> {
    if (retired.get(rowKey) === lastEventAt) return retired;
    const next = new Map(retired);
    next.set(rowKey, lastEventAt);
    return next;
}

/** The terminal rows an auto-linger countdown should run for: not the ones already retired at
 *  this very `lastEventAt`. A countdown armed for a retired row fired again a minute later, retired
 *  it again, and so rebuilt the Swarm list once a minute per retired row. */
export function countdownCandidates(liveTerminal: Map<string, number>, retired: Map<string, number>): Map<string, number> {
    const out = new Map<string, number>();
    for (const [rowKey, lastEventAt] of liveTerminal) {
        if (retired.get(rowKey) !== lastEventAt) out.set(rowKey, lastEventAt);
    }
    return out;
}
