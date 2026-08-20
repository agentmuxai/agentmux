// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import {
    buildCronRows,
    buildDispatchBuckets,
    buildShellRows,
    collectClearableRows,
    filterRetired,
    groupCacheKey,
    hasRenderableBlock,
    loadRetiredRowKeysFromStorage,
    mergeDispatchActivityEntries,
    mergeSubagentsPreservingIdentity,
    pruneGroupIdentityCache,
    pruneRetiredEntries,
    saveRetiredRowKeysToStorage,
    stabilizeGroupIdentity,
    subagentDisplayLabel,
    subagentRowKey,
    type ActiveCron,
    type ActiveShell,
    type ActiveSubagent,
    type AgentDispatch,
    type AgentTreeNode,
    type DispatchActivityEntry,
    type WorkflowDispatch,
} from "./swarm-model";

function mkShell(overrides: Partial<ActiveShell> & Pick<ActiveShell, "shell_id" | "block_id">): ActiveShell {
    return {
        cmd: "npm run dev",
        title: "npm run dev",
        started_at: 0,
        line_count: 0,
        ...overrides,
    };
}

function mkCron(overrides: Partial<ActiveCron> & Pick<ActiveCron, "id" | "block_id">): ActiveCron {
    return {
        name: "nightly build",
        expression: "0 9 * * *",
        target: "target-agent",
        created_by: "creator-agent",
        enabled: true,
        last_fired: null,
        fire_count: 0,
        max_fires: null,
        next_fire: null,
        ...overrides,
    };
}

function mkEntry(opts: { agentId?: string; timestamp: number; content?: string }): DispatchActivityEntry {
    const agentId = opts.agentId ?? "a1";
    return {
        agentId,
        event: {
            agent_id: agentId,
            timestamp: opts.timestamp,
            event_type: { type: "text", content: opts.content ?? "hi" },
        },
    };
}

function mk(overrides: Partial<ActiveSubagent> & Pick<ActiveSubagent, "agent_id">): ActiveSubagent {
    return {
        slug: "",
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        status: "completed",
        spawned_at: 0,
        last_event_at: 0,
        event_count: 1,
        model: null,
        dispatch_id: `solo:${overrides.agent_id}`,
        display_name: null,
        ...overrides,
    };
}

function mkDispatch(overrides: Partial<AgentDispatch> & Pick<AgentDispatch, "dispatch_id">): AgentDispatch {
    return {
        kind: "workflow",
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        member_count: 0,
        members_done: 0,
        status: "running",
        last_event_at: 0,
        dispatch_name: null,
        ...overrides,
    };
}

describe("buildDispatchBuckets", () => {
    it("puts every solo dispatch in agentToolRows, never grouped", () => {
        const { agentToolRows, workflowRows } = buildDispatchBuckets(
            [],
            [mk({ agent_id: "a1" }), mk({ agent_id: "a2" }), mk({ agent_id: "a3" })]
        );
        expect(agentToolRows).toHaveLength(3);
        expect(workflowRows).toHaveLength(0);
    });

    it("renders a Workflow-kind AgentDispatch as one row in workflowRows, regardless of member count", () => {
        const dispatches = [mkDispatch({ dispatch_id: "wf_1", member_count: 3, status: "running" })];
        const subagents = [
            mk({ agent_id: "a1", dispatch_id: "wf_1", slug: "cheerful-enchanting-sketch" }),
            mk({ agent_id: "a2", dispatch_id: "wf_1", slug: "cheerful-enchanting-sketch" }),
            mk({ agent_id: "a3", dispatch_id: "wf_1", slug: "cheerful-enchanting-sketch" }),
        ];
        const { agentToolRows, workflowRows } = buildDispatchBuckets(dispatches, subagents);
        expect(agentToolRows).toHaveLength(0);
        expect(workflowRows).toHaveLength(1);
        expect(workflowRows[0].memberCount).toBe(3);
    });

    it("never renders one row per member — SPEC §7, even for a dispatch this test can't realistically construct at full scale (1,030+ observed live)", () => {
        const dispatches = [mkDispatch({ dispatch_id: "wf_big", member_count: 200 })];
        const subagents = Array.from({ length: 50 }, (_, i) =>
            mk({ agent_id: `m${i}`, dispatch_id: "wf_big" })
        );
        const { workflowRows } = buildDispatchBuckets(dispatches, subagents);
        expect(workflowRows).toHaveLength(1);
        expect(workflowRows[0].memberCount).toBe(200);
    });

    it("does not carry a member list on WorkflowDispatch — SPEC §7 (a large dispatch can't hold thousands of members in the tree atom)", () => {
        const dispatches = [mkDispatch({ dispatch_id: "wf_1", member_count: 2 })];
        const subagents = [mk({ agent_id: "a1", dispatch_id: "wf_1" }), mk({ agent_id: "a2", dispatch_id: "wf_1" })];
        const { workflowRows } = buildDispatchBuckets(dispatches, subagents);
        expect("subagents" in (workflowRows[0] as object)).toBe(false);
        expect("members" in (workflowRows[0] as object)).toBe(false);
    });

    it("mixes agentToolRows and workflowRows independently, one row per distinct dispatch", () => {
        const dispatches = [
            mkDispatch({ dispatch_id: "wf_1", member_count: 2 }),
            mkDispatch({ dispatch_id: "wf_2", member_count: 1 }),
        ];
        const subagents = [
            mk({ agent_id: "a1", dispatch_id: "wf_1" }),
            mk({ agent_id: "a2", dispatch_id: "wf_1" }),
            mk({ agent_id: "a3", dispatch_id: "wf_2" }),
            mk({ agent_id: "a4" }), // solo
            mk({ agent_id: "a5" }), // solo
        ];
        const { agentToolRows, workflowRows } = buildDispatchBuckets(dispatches, subagents);
        expect(workflowRows).toHaveLength(2);
        expect(agentToolRows).toHaveLength(2);
    });

    it("synthesizes a placeholder workflowRows entry for a workflow-kind subagent whose dispatch is missing from `dispatches` (stale/failed ListDispatches) — never spilled into agentToolRows", () => {
        // `loadDispatches()` silently swallows RPC errors, so `dispatches`
        // can lag or miss entries that `subagents` (a separate fetch)
        // already has. A workflow member with no matching AgentDispatch row
        // must degrade to its OWN synthesized workflowRows row (grouped by
        // dispatch_id), not vanish from the tree — and never spill into
        // agentToolRows, which would break the one-row-per-Workflow-call
        // invariant. See SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.1.
        const dispatches = [mkDispatch({ dispatch_id: "wf_1", member_count: 1 })];
        const subagents = [
            mk({ agent_id: "a1", dispatch_id: "wf_1" }),
            mk({ agent_id: "a2", dispatch_id: "wf_missing" }),
        ];
        const { agentToolRows, workflowRows } = buildDispatchBuckets(dispatches, subagents);
        expect(agentToolRows).toHaveLength(0);
        expect(workflowRows).toHaveLength(2);
        const placeholder = workflowRows.find((w) => w.dispatchId === "wf_missing");
        expect(placeholder).toBeDefined();
        expect(placeholder?.memberCount).toBe(1);
    });

    it("groups multiple orphaned members of the SAME missing dispatch into one placeholder row, not one per member", () => {
        const subagents = [
            mk({ agent_id: "a1", dispatch_id: "wf_missing", last_event_at: 100, status: "active" }),
            mk({ agent_id: "a2", dispatch_id: "wf_missing", last_event_at: 200, status: "active" }),
            mk({ agent_id: "a3", dispatch_id: "wf_missing", last_event_at: 150, status: "completed" }),
        ];
        const { agentToolRows, workflowRows } = buildDispatchBuckets([], subagents);
        expect(agentToolRows).toHaveLength(0);
        expect(workflowRows).toHaveLength(1);
        expect(workflowRows[0].dispatchId).toBe("wf_missing");
        expect(workflowRows[0].memberCount).toBe(3);
        expect(workflowRows[0].membersDone).toBe(1);
        expect(workflowRows[0].status).toBe("active");
        expect(workflowRows[0].lastEventAt).toBe(200);
    });

    it("passes AgentDispatch.status through directly — running → active, completed → retired", () => {
        const { workflowRows } = buildDispatchBuckets(
            [
                mkDispatch({ dispatch_id: "wf_1", status: "running" }),
                mkDispatch({ dispatch_id: "wf_2", status: "completed" }),
            ],
            [mk({ agent_id: "a1", dispatch_id: "wf_1" }), mk({ agent_id: "a2", dispatch_id: "wf_2" })]
        );
        const running = workflowRows.find((w) => w.dispatchId === "wf_1");
        const completed = workflowRows.find((w) => w.dispatchId === "wf_2");
        expect(running?.status).toBe("active");
        expect(completed?.status).toBe("retired");
    });

    it("maps an abandoned AgentDispatch.status to 'retired', not 'active' — regression guard for the gap this exact bug would reopen", () => {
        // DispatchStatus::Abandoned (SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md
        // §3.2) is terminal, same as "completed" — a naive `status ===
        // "completed" ? "retired" : "active"` mapping (the pre-fix version
        // of this code) would silently read an abandoned dispatch as still
        // "active", breaking collectClearableRows and the resultPill.
        const { workflowRows } = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_1", status: "abandoned" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_1" })]
        );
        expect(workflowRows[0].status).toBe("retired");
    });

    it("prefers the backend's eager dispatch_name over a member's slug", () => {
        const { workflowRows } = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_1", dispatch_name: "Refactor auth module" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_1", slug: "zesty-crafting-kahan" })]
        );
        expect(workflowRows[0].name).toBe("Refactor auth module");
    });

    it("falls back to a member's slug when dispatch_name hasn't resolved yet", () => {
        const { workflowRows } = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_1", dispatch_name: null })],
            [
                mk({ agent_id: "a1", dispatch_id: "wf_1", slug: "", last_event_at: 200 }),
                mk({ agent_id: "a2", dispatch_id: "wf_1", slug: "zesty-crafting-kahan", last_event_at: 100 }),
            ]
        );
        expect(workflowRows[0].name).toBe("zesty-crafting-kahan");
    });

    it("falls back to the raw dispatch id when neither dispatch_name nor any member slug is available", () => {
        const { workflowRows } = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_unnamed", dispatch_name: null })],
            [mk({ agent_id: "a1", dispatch_id: "wf_unnamed", slug: "" })]
        );
        expect(workflowRows[0].name).toBe("wf_unnamed");
    });

    it("sorts agentToolRows by most recent activity", () => {
        const { agentToolRows } = buildDispatchBuckets(
            [],
            [
                mk({ agent_id: "old", last_event_at: 100 }),
                mk({ agent_id: "newest", last_event_at: 900 }),
                mk({ agent_id: "mid", last_event_at: 500 }),
            ]
        );
        expect(agentToolRows.map((r) => r.agent_id)).toEqual(["newest", "mid", "old"]);
    });

    it("sorts workflowRows by most recent activity", () => {
        const { workflowRows } = buildDispatchBuckets(
            [
                mkDispatch({ dispatch_id: "wf_old", last_event_at: 100 }),
                mkDispatch({ dispatch_id: "wf_newest", last_event_at: 900 }),
                mkDispatch({ dispatch_id: "wf_mid", last_event_at: 500 }),
            ],
            []
        );
        expect(workflowRows.map((r) => r.dispatchId)).toEqual(["wf_newest", "wf_mid", "wf_old"]);
    });

    it("workflow membership always wins over same-slug agentToolRows leakage — a tracked workflow member never also appears in agentToolRows", () => {
        const dispatches = [mkDispatch({ dispatch_id: "wf_1", member_count: 2 })];
        const subagents = [
            mk({ agent_id: "a1", dispatch_id: "wf_1", slug: "shared-slug" }),
            mk({ agent_id: "a2", dispatch_id: "wf_1", slug: "shared-slug" }),
            mk({ agent_id: "a3", slug: "shared-slug" }), // genuinely solo, same slug
        ];
        const { agentToolRows, workflowRows } = buildDispatchBuckets(dispatches, subagents);
        expect(workflowRows).toHaveLength(1);
        expect(agentToolRows).toHaveLength(1);
        expect(agentToolRows[0].agent_id).toBe("a3");
    });
});

describe("mergeSubagentsPreservingIdentity", () => {
    it("reuses the old object reference for an entry whose fields are unchanged", () => {
        const prev = [mk({ agent_id: "a1", slug: "foo" })];
        const next = [mk({ agent_id: "a1", slug: "foo" })];
        const merged = mergeSubagentsPreservingIdentity(prev, next);
        expect(merged[0]).toBe(prev[0]);
    });

    it("uses the new object when a field actually changed", () => {
        const prev = [mk({ agent_id: "a1", status: "active" })];
        const next = [mk({ agent_id: "a1", status: "completed" })];
        const merged = mergeSubagentsPreservingIdentity(prev, next);
        expect(merged[0]).toBe(next[0]);
        expect(merged[0].status).toBe("completed");
    });

    it("only replaces the changed entry, leaving unrelated entries' references untouched", () => {
        const prev = [
            mk({ agent_id: "a1", status: "active" }),
            mk({ agent_id: "a2", status: "active" }),
        ];
        const next = [
            mk({ agent_id: "a1", status: "completed" }), // changed
            mk({ agent_id: "a2", status: "active" }), // unchanged
        ];
        const merged = mergeSubagentsPreservingIdentity(prev, next);
        expect(merged[0]).toBe(next[0]);
        expect(merged[1]).toBe(prev[1]);
    });

    it("uses the new object for a subagent with no prior entry", () => {
        const prev: ActiveSubagent[] = [];
        const next = [mk({ agent_id: "a1" })];
        const merged = mergeSubagentsPreservingIdentity(prev, next);
        expect(merged[0]).toBe(next[0]);
    });
});

describe("stabilizeGroupIdentity", () => {
    it("reuses the cached WorkflowDispatch reference when nothing about the dispatch changed", () => {
        const cache = new Map<string, WorkflowDispatch>();
        const dispatches = [mkDispatch({ dispatch_id: "wf_1", member_count: 1 })];
        const members = [mk({ agent_id: "a1", dispatch_id: "wf_1" })];
        const first = buildDispatchBuckets(dispatches, members).workflowRows;
        const stabilizedFirst = stabilizeGroupIdentity(cache, first);

        // A second, independently-built (but content-identical) row for the
        // same dispatch — mirrors what a fresh buildDispatchBuckets() call
        // produces on an unrelated buildTree() recompute.
        const second = buildDispatchBuckets(dispatches, members).workflowRows;
        const stabilizedSecond = stabilizeGroupIdentity(cache, second);

        expect(stabilizedSecond[0]).toBe(stabilizedFirst[0]);
    });

    it("returns a fresh reference when the dispatch's own fields actually changed", () => {
        const cache = new Map<string, WorkflowDispatch>();
        const first = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_1", member_count: 1, members_done: 0 })],
            [mk({ agent_id: "a1", dispatch_id: "wf_1" })]
        ).workflowRows;
        const stabilizedFirst = stabilizeGroupIdentity(cache, first);

        const second = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_1", member_count: 1, members_done: 1, status: "completed" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_1" })]
        ).workflowRows;
        const stabilizedSecond = stabilizeGroupIdentity(cache, second);

        expect(stabilizedSecond[0]).not.toBe(stabilizedFirst[0]);
    });

    it("keeps two distinct dispatches as independent cache entries", () => {
        const cache = new Map<string, WorkflowDispatch>();
        const rowsA = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_a" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_a" })]
        ).workflowRows;
        const rowsB = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_b" })],
            [mk({ agent_id: "b1", dispatch_id: "wf_b" })]
        ).workflowRows;
        stabilizeGroupIdentity(cache, rowsA);
        stabilizeGroupIdentity(cache, rowsB);
        expect(cache.size).toBe(2);
        expect(cache.has("wf:wf_a")).toBe(true);
        expect(cache.has("wf:wf_b")).toBe(true);
    });
});

describe("mergeDispatchActivityEntries", () => {
    it("appends genuinely new entries and re-sorts by timestamp", () => {
        const prev = [mkEntry({ timestamp: 100 })];
        const merged = mergeDispatchActivityEntries(prev, [mkEntry({ timestamp: 50 }), mkEntry({ timestamp: 200 })]);
        expect(merged.map((e) => e.event.timestamp)).toEqual([50, 100, 200]);
    });

    it("drops an incoming entry that duplicates one already present — the live/backfill race (reagent P1 on #2232)", () => {
        // A solo dispatch's `GetHistory` backfill and the live
        // `dispatch:activity` broadcast can both deliver the same
        // (agentId, timestamp, event) triple if the backend already
        // flushed it before GetHistory resolved.
        const prev = [mkEntry({ timestamp: 100, content: "same" })];
        const merged = mergeDispatchActivityEntries(prev, [mkEntry({ timestamp: 100, content: "same" })]);
        expect(merged).toHaveLength(1);
        expect(merged).toBe(prev); // no-op merge returns the same reference
    });

    it("does not drop two entries that merely share a timestamp but differ in content or agent", () => {
        const prev = [mkEntry({ timestamp: 100, content: "first" })];
        const merged = mergeDispatchActivityEntries(prev, [
            mkEntry({ timestamp: 100, content: "second" }),
            mkEntry({ timestamp: 100, agentId: "a2", content: "first" }),
        ]);
        expect(merged).toHaveLength(3);
    });

    it("caps the merged feed at MAX_DISPATCH_FEED_ENTRIES (500), dropping the oldest first", () => {
        const prev = Array.from({ length: 500 }, (_, i) => mkEntry({ timestamp: i }));
        const merged = mergeDispatchActivityEntries(prev, [mkEntry({ timestamp: 1000 })]);
        expect(merged).toHaveLength(500);
        expect(merged[0].event.timestamp).toBe(1);
        expect(merged[merged.length - 1].event.timestamp).toBe(1000);
    });

    it("returns the input `prev` array unchanged (by reference) when incoming is empty", () => {
        const prev = [mkEntry({ timestamp: 100 })];
        expect(mergeDispatchActivityEntries(prev, [])).toBe(prev);
    });
});

describe("groupCacheKey", () => {
    it("namespaces every key with 'wf:'", () => {
        const [wf] = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_1" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_1" })]
        ).workflowRows;
        expect(groupCacheKey(wf)).toBe("wf:wf_1");
    });
});

describe("subagentRowKey", () => {
    it("prefixes with 'agent:'", () => {
        expect(subagentRowKey("abc123")).toBe("agent:abc123");
    });
});

function mkNode(agentToolRows: ActiveSubagent[], workflowRows: WorkflowDispatch[]): AgentTreeNode {
    return {
        blockId: "block-1",
        agentName: "Agent",
        agentProvider: null,
        activitySummary: null,
        contextTokens: null,
        agentStatus: "idle",
        agentToolRows,
        workflowRows,
        shellRows: [],
        cronRows: [],
    };
}

describe("collectClearableRows", () => {
    it("includes a terminal-status (completed) Agent Tool row", () => {
        const node = mkNode([mk({ agent_id: "a1", status: "completed", last_event_at: 500 })], []);
        const rows = collectClearableRows([node]);
        expect(rows).toEqual([{ rowKey: "agent:a1", lastEventAt: 500 }]);
    });

    it("includes an abandoned Agent Tool row", () => {
        const node = mkNode([mk({ agent_id: "a1", status: "abandoned", last_event_at: 300 })], []);
        expect(collectClearableRows([node])).toEqual([{ rowKey: "agent:a1", lastEventAt: 300 }]);
    });

    it("excludes a still-active Agent Tool row", () => {
        const node = mkNode([mk({ agent_id: "a1", status: "active", last_event_at: 500 })], []);
        expect(collectClearableRows([node])).toEqual([]);
    });

    it("includes a terminal ('retired', i.e. all-members-done) WorkflowDispatch row", () => {
        const [wf] = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_1", status: "completed", last_event_at: 700 })],
            [mk({ agent_id: "a1", dispatch_id: "wf_1" })]
        ).workflowRows;
        const rows = collectClearableRows([mkNode([], [wf])]);
        expect(rows).toEqual([{ rowKey: "wf_1", lastEventAt: 700 }]);
    });

    it("excludes a still-running WorkflowDispatch row", () => {
        const [wf] = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_1", status: "running" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_1" })]
        ).workflowRows;
        expect(collectClearableRows([mkNode([], [wf])])).toEqual([]);
    });

    it("collects across multiple blocks/nodes", () => {
        const nodeA = mkNode([mk({ agent_id: "a1", status: "completed", last_event_at: 1 })], []);
        const nodeB = mkNode([mk({ agent_id: "b1", status: "abandoned", last_event_at: 2 })], []);
        expect(collectClearableRows([nodeA, nodeB])).toHaveLength(2);
    });

    it("returns an empty array when nothing is clearable", () => {
        expect(collectClearableRows([mkNode([], [])])).toEqual([]);
    });
});

describe("filterRetired", () => {
    const keyFn = (n: number) => `row-${n}`;

    it("keeps a row whose key is not in the retired map", () => {
        const retired = new Map<string, number>();
        const rows = filterRetired([1, 2], retired, keyFn, () => 100);
        expect(rows).toEqual([1, 2]);
    });

    it("drops a row whose key is retired and lastEventAt still matches the retired snapshot", () => {
        const retired = new Map<string, number>([["row-1", 100]]);
        const rows = filterRetired([1, 2], retired, keyFn, () => 100);
        expect(rows).toEqual([2]);
    });

    it("un-retires automatically once the row's own lastEventAt moves past the retired snapshot", () => {
        // Same key as a retired row, but new activity has since advanced
        // lastEventAt past what was snapshotted at retire time — SPEC_
        // SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20 §6: no
        // separate un-retire action needed, the row just reappears.
        const retired = new Map<string, number>([["row-1", 100]]);
        const rows = filterRetired([1], retired, keyFn, () => 200);
        expect(rows).toEqual([1]);
    });

    it("is a no-op (identity semantics aside) when nothing is retired", () => {
        const rows = filterRetired([1, 2, 3], new Map(), keyFn, () => 0);
        expect(rows).toEqual([1, 2, 3]);
    });
});

describe("pruneRetiredEntries", () => {
    it("drops entries whose key is no longer in liveKeys", () => {
        const retired = new Map<string, number>([
            ["agent:a1", 100],
            ["agent:a2", 200],
        ]);
        const pruned = pruneRetiredEntries(retired, new Set(["agent:a1"]));
        expect(pruned.has("agent:a1")).toBe(true);
        expect(pruned.has("agent:a2")).toBe(false);
    });

    it("keeps every entry whose key is still live, even if not un-retired yet", () => {
        // Still-live rows keep their retired entry regardless of this pass —
        // only pruneRetiredEntries' own liveness check matters here, not
        // filterRetired's separate lastEventAt comparison.
        const retired = new Map<string, number>([["agent:a1", 100]]);
        const pruned = pruneRetiredEntries(retired, new Set(["agent:a1"]));
        expect(pruned.get("agent:a1")).toBe(100);
    });

    it("returns the same Map reference when nothing was dropped", () => {
        const retired = new Map<string, number>([["agent:a1", 100]]);
        const pruned = pruneRetiredEntries(retired, new Set(["agent:a1"]));
        expect(pruned).toBe(retired);
    });

    it("returns a new Map when something was dropped", () => {
        const retired = new Map<string, number>([["agent:gone", 100]]);
        const pruned = pruneRetiredEntries(retired, new Set());
        expect(pruned).not.toBe(retired);
        expect(pruned.size).toBe(0);
    });
});

describe("loadRetiredRowKeysFromStorage / saveRetiredRowKeysToStorage", () => {
    beforeEach(() => {
        localStorage.clear();
    });

    it("round-trips a map through localStorage", () => {
        const retired = new Map<string, number>([
            ["agent:a1", 100],
            ["wf_1", 200],
        ]);
        saveRetiredRowKeysToStorage(retired);
        const loaded = loadRetiredRowKeysFromStorage();
        expect(loaded).toEqual(retired);
    });

    it("returns an empty map when nothing has been saved yet", () => {
        expect(loadRetiredRowKeysFromStorage()).toEqual(new Map());
    });

    it("returns an empty map (not a throw) when the stored value is corrupt JSON", () => {
        localStorage.setItem("agentmux:swarm-retired-rows", "{not valid json");
        expect(loadRetiredRowKeysFromStorage()).toEqual(new Map());
    });

    it("overwrites the previously-saved value on a second save", () => {
        saveRetiredRowKeysToStorage(new Map([["agent:a1", 100]]));
        saveRetiredRowKeysToStorage(new Map([["agent:a2", 200]]));
        const loaded = loadRetiredRowKeysFromStorage();
        expect(loaded.has("agent:a1")).toBe(false);
        expect(loaded.get("agent:a2")).toBe(200);
    });
});

describe("pruneGroupIdentityCache", () => {
    it("drops entries for dispatches no longer live, keeps the rest", () => {
        const cache = new Map<string, WorkflowDispatch>();
        const groupA = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_a" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_a" })]
        ).workflowRows;
        const groupB = buildDispatchBuckets(
            [mkDispatch({ dispatch_id: "wf_b" })],
            [mk({ agent_id: "b1", dispatch_id: "wf_b" })]
        ).workflowRows;
        stabilizeGroupIdentity(cache, groupA);
        stabilizeGroupIdentity(cache, groupB);
        expect(cache.size).toBe(2);

        // Live keys use groupCacheKey's namespaced "wf:<id>" form, not the
        // raw dispatchId.
        pruneGroupIdentityCache(cache, new Set(["wf:wf_a"]));
        expect(cache.size).toBe(1);
        expect(cache.has("wf:wf_a")).toBe(true);
        expect(cache.has("wf:wf_b")).toBe(false);
    });
});

describe("subagentDisplayLabel", () => {
    it("prefers display_name over everything else", () => {
        expect(
            subagentDisplayLabel({ display_name: "Refactor shell module", slug: "cheerful-enchanting-sketch", agent_id: "abc1234def" })
        ).toBe("Refactor shell module");
    });

    it("falls back to agent_id short prefix when there is no slug either", () => {
        expect(subagentDisplayLabel({ display_name: null, slug: "", agent_id: "abc1234def" })).toBe("abc1234");
    });

    // Regression for task #44's live repro: 17 structurally distinct
    // agent_ids spawned within ~50ms of one another under one parent all
    // resolved the identical literal slug "magical-enchanting-diffie" —
    // confirmed (see subagentDisplayLabel's doc comment) to be a shared
    // Claude Code per-session/per-batch codename, not a per-subagent unique
    // value, so a bare `slug` fallback rendered all 17 as visually
    // identical rows before any per-agent display_name had resolved.
    it("disambiguates same-slug siblings with a short agent_id suffix instead of returning a bare, non-unique slug", () => {
        const shared = "magical-enchanting-diffie";
        // The short-id disambiguator is `agent_id.substring(0, 7)` — pad the
        // index so it lands within the first 7 characters, otherwise every
        // id would collide on the same "agent-0" prefix and this test would
        // fail to exercise the actual disambiguation.
        const subagents = Array.from({ length: 17 }, (_, i) => ({
            display_name: null as string | null,
            slug: shared,
            agent_id: `${i.toString().padStart(7, "0")}-agent-${"x".repeat(10)}`,
        }));

        const labels = subagents.map((s) => subagentDisplayLabel(s));

        // Every label still surfaces the shared slug (it's real, useful
        // context — this batch legitimately shares a codename)...
        expect(labels.every((l) => l.startsWith(`${shared} · `))).toBe(true);
        // ...but all 17 labels are nonetheless pairwise distinct, since each
        // subagent's own agent_id is genuinely unique.
        expect(new Set(labels).size).toBe(17);
    });

    it("still includes the short agent_id suffix even when slug looks unique in isolation", () => {
        // subagentDisplayLabel can't know whether a slug collides with a
        // sibling's without seeing the whole list, so it always
        // disambiguates when falling back to slug rather than only on
        // detected collision — cheap, consistent, and never wrong.
        expect(subagentDisplayLabel({ display_name: null, slug: "solo-run", agent_id: "zzzz999yyyy" })).toBe(
            "solo-run · zzzz999"
        );
    });
});

describe("buildShellRows", () => {
    it("keeps only shells matching the given block_id", () => {
        const shells = [
            mkShell({ shell_id: "s1", block_id: "block-a" }),
            mkShell({ shell_id: "s2", block_id: "block-b" }),
            mkShell({ shell_id: "s3", block_id: "block-a" }),
        ];
        const rows = buildShellRows(shells, "block-a");
        expect(new Set(rows.map((r) => r.shell_id))).toEqual(new Set(["s1", "s3"]));
        expect(rows.every((r) => r.block_id === "block-a")).toBe(true);
    });

    it("sorts newest-first by started_at", () => {
        const shells = [
            mkShell({ shell_id: "old", block_id: "block-a", started_at: 100 }),
            mkShell({ shell_id: "new", block_id: "block-a", started_at: 300 }),
            mkShell({ shell_id: "mid", block_id: "block-a", started_at: 200 }),
        ];
        const rows = buildShellRows(shells, "block-a");
        expect(rows.map((r) => r.shell_id)).toEqual(["new", "mid", "old"]);
    });

    it("returns an empty array when no shells match", () => {
        const shells = [mkShell({ shell_id: "s1", block_id: "block-a" })];
        expect(buildShellRows(shells, "block-z")).toEqual([]);
    });

    it("returns an empty array for a null blockId", () => {
        const shells = [mkShell({ shell_id: "s1", block_id: "block-a" })];
        expect(buildShellRows(shells, null)).toEqual([]);
    });
});

describe("buildCronRows", () => {
    it("keeps only cron jobs matching the given block_id", () => {
        const crons = [
            mkCron({ id: "c1", block_id: "block-a" }),
            mkCron({ id: "c2", block_id: "block-b" }),
            mkCron({ id: "c3", block_id: "block-a" }),
        ];
        const rows = buildCronRows(crons, "block-a");
        expect(new Set(rows.map((r) => r.id))).toEqual(new Set(["c1", "c3"]));
        expect(rows.every((r) => r.block_id === "block-a")).toBe(true);
    });

    it("sorts newest-last_fired-first", () => {
        const crons = [
            mkCron({ id: "old", block_id: "block-a", last_fired: 100 }),
            mkCron({ id: "new", block_id: "block-a", last_fired: 300 }),
            mkCron({ id: "mid", block_id: "block-a", last_fired: 200 }),
        ];
        const rows = buildCronRows(crons, "block-a");
        expect(rows.map((r) => r.id)).toEqual(["new", "mid", "old"]);
    });

    it("sorts a never-fired (null last_fired) job last", () => {
        const crons = [
            mkCron({ id: "never", block_id: "block-a", last_fired: null }),
            mkCron({ id: "fired", block_id: "block-a", last_fired: 100 }),
        ];
        const rows = buildCronRows(crons, "block-a");
        expect(rows.map((r) => r.id)).toEqual(["fired", "never"]);
    });

    it("returns an empty array when no cron jobs match", () => {
        const crons = [mkCron({ id: "c1", block_id: "block-a" })];
        expect(buildCronRows(crons, "block-z")).toEqual([]);
    });

    it("returns an empty array for a null blockId", () => {
        const crons = [mkCron({ id: "c1", block_id: "block-a" })];
        expect(buildCronRows(crons, null)).toEqual([]);
    });
});

describe("hasRenderableBlock", () => {
    it("is false for undefined once loading has resolved — the case a genuinely orphaned parent_block_id reaches", () => {
        expect(hasRenderableBlock(undefined, false)).toBe(false);
    });

    it("is false for null once loading has resolved", () => {
        expect(hasRenderableBlock(null, false)).toBe(false);
    });

    it("is true for a real block object, even one with no meta set yet", () => {
        expect(hasRenderableBlock({ oid: "block-a", meta: {} }, false)).toBe(true);
    });

    it("is true for undefined while still loading — a fresh WOS oref reads as null until GetObject resolves, indistinguishable from a phantom by value alone (reagentx P1 on #2438)", () => {
        expect(hasRenderableBlock(undefined, true)).toBe(true);
    });

    it("is true for null while still loading, same reasoning", () => {
        expect(hasRenderableBlock(null, true)).toBe(true);
    });

    it("is true for a real block while (implausibly) still marked loading — loading state never overrides a genuinely present value", () => {
        expect(hasRenderableBlock({ oid: "block-a", meta: {} }, true)).toBe(true);
    });
});
