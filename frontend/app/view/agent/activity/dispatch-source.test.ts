// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { AgentDispatch } from "../../swarm/swarm-model";
import { msUntilNextQuietWindowRefresh } from "./dispatch-source";

function mkDispatch(overrides: Partial<AgentDispatch> & Pick<AgentDispatch, "dispatch_id">): AgentDispatch {
    return {
        kind: "solo",
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        member_count: 1,
        members_done: 0,
        status: "running",
        last_event_at: 0,
        dispatch_name: null,
        ...overrides,
    } as AgentDispatch;
}

describe("msUntilNextQuietWindowRefresh", () => {
    it("returns null when nothing is counts-complete", () => {
        const d = mkDispatch({ dispatch_id: "d1", member_count: 2, members_done: 1, status: "running" });
        expect(msUntilNextQuietWindowRefresh([d], 0)).toBeNull();
    });

    it("returns null for a dispatch already marked completed (not still 'running')", () => {
        // "abandoned" is a separate, not-yet-merged PR's addition to
        // AgentDispatch.status (SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md)
        // — irrelevant to this predicate either way, since it only ever
        // triggers on status === "running".
        const completed = mkDispatch({ dispatch_id: "d1", member_count: 1, members_done: 1, status: "completed" });
        expect(msUntilNextQuietWindowRefresh([completed], 0)).toBeNull();
    });

    it("returns the ms remaining until the 60s quiet window elapses for a counts-complete-but-still-running dispatch", () => {
        const d = mkDispatch({ dispatch_id: "d1", member_count: 1, members_done: 1, status: "running", last_event_at: 1000 });
        expect(msUntilNextQuietWindowRefresh([d], 1000 + 40_000)).toBe(20_000);
    });

    it("floors at 0 once the deadline has already passed", () => {
        const d = mkDispatch({ dispatch_id: "d1", member_count: 1, members_done: 1, status: "running", last_event_at: 1000 });
        expect(msUntilNextQuietWindowRefresh([d], 1000 + 90_000)).toBe(0);
    });

    it("picks the EARLIEST deadline among multiple pending dispatches", () => {
        const soon = mkDispatch({ dispatch_id: "d1", member_count: 1, members_done: 1, status: "running", last_event_at: 50_000 });
        const later = mkDispatch({ dispatch_id: "d2", member_count: 1, members_done: 1, status: "running", last_event_at: 10_000 });
        // soon's deadline: 50_000 + 60_000 = 110_000; later's: 10_000 + 60_000 = 70_000 — later is earlier in absolute terms.
        expect(msUntilNextQuietWindowRefresh([soon, later], 0)).toBe(70_000);
    });

    it("ignores a dispatch with member_count 0 (nothing to be complete about)", () => {
        const d = mkDispatch({ dispatch_id: "d1", member_count: 0, members_done: 0, status: "running" });
        expect(msUntilNextQuietWindowRefresh([d], 0)).toBeNull();
    });
});
