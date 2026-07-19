// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    buildDispatchChildren,
    groupCacheKey,
    isNameGroup,
    isWorkflowDispatch,
    mergeSubagentsPreservingIdentity,
    pruneGroupIdentityCache,
    stabilizeGroupIdentity,
    subagentDisplayLabel,
    type ActiveSubagent,
    type AgentDispatch,
    type NameGroup,
    type SwarmChild,
    type WorkflowDispatch,
} from "./swarm-model";

/** Extract a SwarmChild's own identifying id, for assertions that don't care
 *  which of the three variants they got — narrows through all three via
 *  isWorkflowDispatch/isNameGroup rather than an unsafe cast. */
function childId(c: SwarmChild): string {
    if (isWorkflowDispatch(c)) return c.dispatchId;
    if (isNameGroup(c)) return c.name;
    return c.agent_id;
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
        ...overrides,
    };
}

describe("buildDispatchChildren", () => {
    it("leaves solo-dispatch subagents (no tracked AgentDispatch) as loose, ungrouped rows", () => {
        const result = buildDispatchChildren([], [mk({ agent_id: "a1" }), mk({ agent_id: "a2" })]);
        expect(result).toHaveLength(2);
        expect(result.every((c) => !isWorkflowDispatch(c))).toBe(true);
    });

    it("renders a Workflow-kind AgentDispatch as one row regardless of member count", () => {
        const dispatches = [mkDispatch({ dispatch_id: "wf_1", member_count: 3, status: "running" })];
        const subagents = [
            mk({ agent_id: "a1", dispatch_id: "wf_1", slug: "cheerful-enchanting-sketch" }),
            mk({ agent_id: "a2", dispatch_id: "wf_1", slug: "cheerful-enchanting-sketch" }),
            mk({ agent_id: "a3", dispatch_id: "wf_1", slug: "cheerful-enchanting-sketch" }),
        ];
        const result = buildDispatchChildren(dispatches, subagents);
        expect(result).toHaveLength(1);
        const group = result[0];
        expect(isWorkflowDispatch(group)).toBe(true);
        if (isWorkflowDispatch(group)) {
            expect(group.memberCount).toBe(3);
            expect(group.name).toBe("cheerful-enchanting-sketch");
        }
    });

    it("never renders one row per member — SPEC §7, even for a dispatch this test can't realistically construct at full scale (1,030+ observed live)", () => {
        const dispatches = [mkDispatch({ dispatch_id: "wf_big", member_count: 200 })];
        const subagents = Array.from({ length: 50 }, (_, i) =>
            mk({ agent_id: `m${i}`, dispatch_id: "wf_big" })
        );
        const result = buildDispatchChildren(dispatches, subagents);
        // One row for the dispatch — the (partial, in this test) member list
        // never expands into its own rows.
        expect(result).toHaveLength(1);
        expect(isWorkflowDispatch(result[0])).toBe(true);
    });

    it("does not carry a member list on WorkflowDispatch — SPEC §7 (a large dispatch can't hold thousands of members in the tree atom)", () => {
        const dispatches = [mkDispatch({ dispatch_id: "wf_1", member_count: 2 })];
        const subagents = [mk({ agent_id: "a1", dispatch_id: "wf_1" }), mk({ agent_id: "a2", dispatch_id: "wf_1" })];
        const [group] = buildDispatchChildren(dispatches, subagents);
        expect("subagents" in (group as object)).toBe(false);
    });

    it("mixes loose subagents and workflow dispatches independently, one row per distinct dispatch", () => {
        const dispatches = [
            mkDispatch({ dispatch_id: "wf_1", member_count: 2 }),
            mkDispatch({ dispatch_id: "wf_2", member_count: 1 }),
        ];
        const subagents = [
            mk({ agent_id: "a1", dispatch_id: "wf_1" }),
            mk({ agent_id: "a2", dispatch_id: "wf_1" }),
            mk({ agent_id: "a3", dispatch_id: "wf_2" }),
            mk({ agent_id: "a4" }), // solo
        ];
        const result = buildDispatchChildren(dispatches, subagents);
        const groups = result.filter(isWorkflowDispatch);
        const loose = result.filter((c) => !isWorkflowDispatch(c));
        expect(groups).toHaveLength(2);
        expect(loose).toHaveLength(1);
    });

    it("falls back to a loose row for a workflow-kind subagent whose dispatch is missing from `dispatches` (stale/failed ListDispatches)", () => {
        // `loadDispatches()` silently swallows RPC errors, so `dispatches`
        // can lag or miss entries that `subagents` (a separate fetch)
        // already has. A workflow member with no matching AgentDispatch row
        // must degrade to a loose row, not vanish from the tree.
        const dispatches = [mkDispatch({ dispatch_id: "wf_1", member_count: 1 })];
        const subagents = [
            mk({ agent_id: "a1", dispatch_id: "wf_1" }),
            mk({ agent_id: "a2", dispatch_id: "wf_missing" }),
        ];
        const result = buildDispatchChildren(dispatches, subagents);
        const groups = result.filter(isWorkflowDispatch);
        const loose = result.filter((c) => !isWorkflowDispatch(c));
        expect(groups).toHaveLength(1);
        expect(loose).toHaveLength(1);
        expect(childId(loose[0])).toBe("a2");
    });

    it("passes AgentDispatch.status through directly — running → active, completed → retired", () => {
        const running = buildDispatchChildren(
            [mkDispatch({ dispatch_id: "wf_1", status: "running" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_1" })]
        )[0];
        const completed = buildDispatchChildren(
            [mkDispatch({ dispatch_id: "wf_2", status: "completed" })],
            [mk({ agent_id: "a2", dispatch_id: "wf_2" })]
        )[0];
        if (isWorkflowDispatch(running) && isWorkflowDispatch(completed)) {
            expect(running.status).toBe("active");
            expect(completed.status).toBe("retired");
        } else {
            throw new Error("expected workflow dispatch rows");
        }
    });

    it("derives the dispatch name from the first member with a non-empty slug", () => {
        const result = buildDispatchChildren(
            [mkDispatch({ dispatch_id: "wf_1" })],
            [
                mk({ agent_id: "a1", dispatch_id: "wf_1", slug: "", last_event_at: 200 }),
                mk({ agent_id: "a2", dispatch_id: "wf_1", slug: "zesty-crafting-kahan", last_event_at: 100 }),
            ]
        );
        const group = result[0];
        if (isWorkflowDispatch(group)) {
            expect(group.name).toBe("zesty-crafting-kahan");
        } else {
            throw new Error("expected a workflow dispatch");
        }
    });

    it("falls back to the dispatch id as the name when no member has a slug (or no members are known yet)", () => {
        const result = buildDispatchChildren(
            [mkDispatch({ dispatch_id: "wf_unnamed" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_unnamed", slug: "" })]
        );
        const group = result[0];
        if (isWorkflowDispatch(group)) {
            expect(group.name).toBe("wf_unnamed");
        } else {
            throw new Error("expected a workflow dispatch");
        }
    });

    it("sorts loose subagents and dispatches together by most recent activity", () => {
        const result = buildDispatchChildren(
            [mkDispatch({ dispatch_id: "wf_1", last_event_at: 500 })],
            [
                mk({ agent_id: "old-loose", last_event_at: 100 }),
                mk({ agent_id: "a1", dispatch_id: "wf_1", last_event_at: 500 }),
                mk({ agent_id: "newest-loose", last_event_at: 900 }),
            ]
        );
        expect(result).toHaveLength(3);
        // newest-loose (900) > wf_1 dispatch (500) > old-loose (100)
        expect(childId(result[0])).toBe("newest-loose");
        expect(childId(result[2])).toBe("old-loose");
    });

    describe("name-based grouping of solo dispatches (issue: same-name duplicate rows)", () => {
        it("collapses 2+ solo dispatches sharing a display_name into one NameGroup", () => {
            const result = buildDispatchChildren(
                [],
                [
                    mk({ agent_id: "a1", display_name: "Code Reviewer" }),
                    mk({ agent_id: "a2", display_name: "Code Reviewer" }),
                    mk({ agent_id: "a3", display_name: "Code Reviewer" }),
                ]
            );
            expect(result).toHaveLength(1);
            const group = result[0];
            expect(isNameGroup(group)).toBe(true);
            if (isNameGroup(group)) {
                expect(group.name).toBe("Code Reviewer");
                expect(group.totalCount).toBe(3);
            }
        });

        it("leaves a single subagent with a unique display_name as a loose row (no group chrome for a non-dupe)", () => {
            const result = buildDispatchChildren([], [mk({ agent_id: "a1", display_name: "Only One" })]);
            expect(result).toHaveLength(1);
            expect(isNameGroup(result[0])).toBe(false);
            expect(childId(result[0])).toBe("a1");
        });

        it("leaves subagents with no display_name AND no slug as loose, ungrouped rows even if several exist", () => {
            const result = buildDispatchChildren(
                [],
                [mk({ agent_id: "a1", display_name: null }), mk({ agent_id: "a2", display_name: null })]
            );
            expect(result).toHaveLength(2);
            expect(result.every((c) => !isNameGroup(c))).toBe(true);
        });

        it("collapses 2+ solo dispatches sharing only a slug (no display_name resolved yet) into one NameGroup", () => {
            // The common case: Claude Code stamps one slug per CLI session on
            // every subagent it spawns (not per-subagent), and display_name
            // only resolves once a client manually expands a row — so most
            // solo dispatches are seen here with display_name: null and an
            // identical slug. Without the slug fallback, a session with many
            // solo Task-tool calls renders one row per call at the top level,
            // which is the "dozens of copies of the same slug" regression.
            const result = buildDispatchChildren(
                [],
                [
                    mk({ agent_id: "a1", display_name: null, slug: "quizzical-tumbling-valiant" }),
                    mk({ agent_id: "a2", display_name: null, slug: "quizzical-tumbling-valiant" }),
                    mk({ agent_id: "a3", display_name: null, slug: "quizzical-tumbling-valiant" }),
                ]
            );
            expect(result).toHaveLength(1);
            const group = result[0];
            expect(isNameGroup(group)).toBe(true);
            if (isNameGroup(group)) {
                expect(group.name).toBe("quizzical-tumbling-valiant");
                expect(group.totalCount).toBe(3);
            }
        });

        it("prefers display_name over slug as the grouping key — a named member does not join its same-slug siblings' slug-keyed group", () => {
            const result = buildDispatchChildren(
                [],
                [
                    mk({ agent_id: "a1", display_name: null, slug: "shared-slug" }),
                    mk({ agent_id: "a2", display_name: null, slug: "shared-slug" }),
                    mk({ agent_id: "a3", display_name: "Named Differently", slug: "shared-slug" }),
                ]
            );
            const groups = result.filter(isNameGroup);
            // a1/a2 collapse under the shared slug; a3 (resolved display_name)
            // is its own loose row since nothing else shares "Named Differently".
            expect(groups).toHaveLength(1);
            expect(groups[0].name).toBe("shared-slug");
            expect(groups[0].totalCount).toBe(2);
            expect(result.some((c) => !isNameGroup(c) && !isWorkflowDispatch(c) && c.agent_id === "a3")).toBe(true);
        });

        it("workflow dispatches take priority — same-named subagents in a workflow never form a NameGroup", () => {
            const result = buildDispatchChildren(
                [mkDispatch({ dispatch_id: "wf_1", member_count: 2 })],
                [
                    mk({ agent_id: "a1", dispatch_id: "wf_1", display_name: "Worker" }),
                    mk({ agent_id: "a2", dispatch_id: "wf_1", display_name: "Worker" }),
                ]
            );
            expect(result).toHaveLength(1);
            expect(isWorkflowDispatch(result[0])).toBe(true);
            expect(result.some(isNameGroup)).toBe(false);
        });

        it("marks a NameGroup active if any member is still active", () => {
            const result = buildDispatchChildren(
                [],
                [
                    mk({ agent_id: "a1", display_name: "N", status: "completed" }),
                    mk({ agent_id: "a2", display_name: "N", status: "active" }),
                ]
            );
            const group = result[0];
            if (isNameGroup(group)) {
                expect(group.status).toBe("active");
                expect(group.activeCount).toBe(1);
            } else {
                throw new Error("expected a name group");
            }
        });

        it("marks a NameGroup retired only once every member has completed", () => {
            const result = buildDispatchChildren(
                [],
                [
                    mk({ agent_id: "a1", display_name: "N", status: "completed" }),
                    mk({ agent_id: "a2", display_name: "N", status: "completed" }),
                ]
            );
            const group = result[0];
            if (isNameGroup(group)) {
                expect(group.status).toBe("retired");
            } else {
                throw new Error("expected a name group");
            }
        });

        it("treats an abandoned NameGroup member the same as a completed one (both terminal)", () => {
            const result = buildDispatchChildren(
                [],
                [
                    mk({ agent_id: "a1", display_name: "N", status: "completed" }),
                    mk({ agent_id: "a2", display_name: "N", status: "abandoned" }),
                ]
            );
            const group = result[0];
            if (isNameGroup(group)) {
                expect(group.status).toBe("retired");
                expect(group.activeCount).toBe(0);
            } else {
                throw new Error("expected a name group");
            }
        });

        it("mixes loose subagents, workflow dispatches, and name groups together, sorted by recency", () => {
            const result = buildDispatchChildren(
                [mkDispatch({ dispatch_id: "wf_1", last_event_at: 500 })],
                [
                    mk({ agent_id: "solo", display_name: "Unique", last_event_at: 50 }),
                    mk({ agent_id: "n1", display_name: "Dup", last_event_at: 300 }),
                    mk({ agent_id: "n2", display_name: "Dup", last_event_at: 700 }),
                    mk({ agent_id: "w1", dispatch_id: "wf_1", last_event_at: 500 }),
                ]
            );
            expect(result).toHaveLength(3);
            const nameGroups = result.filter(isNameGroup);
            const workflowDispatches = result.filter(isWorkflowDispatch);
            expect(nameGroups).toHaveLength(1);
            expect(workflowDispatches).toHaveLength(1);
            // Dup group's lastEventAt (700, from n2) > wf_1 dispatch (500) > solo (50)
            expect(childId(result[0])).toBe("Dup");
            expect(childId(result[2])).toBe("solo");
        });
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
        const first = buildDispatchChildren(dispatches, members);
        const stabilizedFirst = stabilizeGroupIdentity(cache, first);

        // A second, independently-built (but content-identical) row for the
        // same dispatch — mirrors what a fresh buildDispatchChildren() call
        // produces on an unrelated buildTree() recompute.
        const second = buildDispatchChildren(dispatches, members);
        const stabilizedSecond = stabilizeGroupIdentity(cache, second);

        expect(stabilizedSecond[0]).toBe(stabilizedFirst[0]);
    });

    it("returns a fresh reference when the dispatch's own fields actually changed", () => {
        const cache = new Map<string, WorkflowDispatch>();
        const first = buildDispatchChildren(
            [mkDispatch({ dispatch_id: "wf_1", member_count: 1, members_done: 0 })],
            [mk({ agent_id: "a1", dispatch_id: "wf_1" })]
        );
        const stabilizedFirst = stabilizeGroupIdentity(cache, first);

        const second = buildDispatchChildren(
            [mkDispatch({ dispatch_id: "wf_1", member_count: 1, members_done: 1, status: "completed" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_1" })]
        );
        const stabilizedSecond = stabilizeGroupIdentity(cache, second);

        expect(stabilizedSecond[0]).not.toBe(stabilizedFirst[0]);
    });

    it("passes loose (non-group) subagents through unchanged", () => {
        const cache = new Map<string, WorkflowDispatch>();
        const loose = buildDispatchChildren([], [mk({ agent_id: "a1" })]);
        const stabilized = stabilizeGroupIdentity(cache, loose);
        expect(stabilized[0]).toBe(loose[0]);
        expect(cache.size).toBe(0);
    });

    it("reuses the cached NameGroup reference when nothing about the group changed", () => {
        const cache = new Map<string, WorkflowDispatch | NameGroup>();
        const members = [
            mk({ agent_id: "a1", display_name: "Dup" }),
            mk({ agent_id: "a2", display_name: "Dup" }),
        ];
        const first = buildDispatchChildren([], members);
        const stabilizedFirst = stabilizeGroupIdentity(cache, first);

        const second = buildDispatchChildren([], members);
        const stabilizedSecond = stabilizeGroupIdentity(cache, second);

        expect(stabilizedSecond[0]).toBe(stabilizedFirst[0]);
    });

    it("keeps a WorkflowDispatch and a NameGroup with the same raw id/name as distinct cache entries", () => {
        // groupCacheKey namespaces with "wf:"/"name:" precisely so this
        // coincidence can't collide two unrelated groups into one cache slot.
        const cache = new Map<string, WorkflowDispatch | NameGroup>();
        const wfChildren = buildDispatchChildren(
            [mkDispatch({ dispatch_id: "shared-id", member_count: 2 })],
            [mk({ agent_id: "a1", dispatch_id: "shared-id" }), mk({ agent_id: "a2", dispatch_id: "shared-id" })]
        );
        const nameChildren = buildDispatchChildren(
            [],
            [
                mk({ agent_id: "b1", display_name: "shared-id" }),
                mk({ agent_id: "b2", display_name: "shared-id" }),
            ]
        );
        stabilizeGroupIdentity(cache, wfChildren);
        stabilizeGroupIdentity(cache, nameChildren);
        expect(cache.size).toBe(2);
        expect(cache.has("wf:shared-id")).toBe(true);
        expect(cache.has("name:block-1:shared-id")).toBe(true);
    });

    it("keeps same-named NameGroups from two different agent blocks as distinct cache entries (reagent P1 on #2123)", () => {
        // groupIdentityCache/expandedIds are shared across the WHOLE tree
        // (buildTree() calls buildDispatchChildren once per block, into one
        // cache) — a Haiku-generated name like "Code Reviewer" can plausibly
        // repeat across two unrelated agent panes. Without parentBlockId in
        // the key, block A's and block B's same-named groups would stomp
        // each other's identity/expand state.
        const cache = new Map<string, WorkflowDispatch | NameGroup>();
        const blockAChildren = buildDispatchChildren(
            [],
            [
                mk({ agent_id: "a1", parent_block_id: "block-A", display_name: "Code Reviewer" }),
                mk({ agent_id: "a2", parent_block_id: "block-A", display_name: "Code Reviewer" }),
            ]
        );
        const blockBChildren = buildDispatchChildren(
            [],
            [
                mk({ agent_id: "b1", parent_block_id: "block-B", display_name: "Code Reviewer" }),
                mk({ agent_id: "b2", parent_block_id: "block-B", display_name: "Code Reviewer" }),
            ]
        );
        const stabilizedA = stabilizeGroupIdentity(cache, blockAChildren);
        const stabilizedB = stabilizeGroupIdentity(cache, blockBChildren);
        expect(cache.size).toBe(2);
        expect(stabilizedA[0]).not.toBe(stabilizedB[0]);
        if (isNameGroup(stabilizedA[0]) && isNameGroup(stabilizedB[0])) {
            expect(stabilizedA[0].subagents.map((s) => s.agent_id)).toEqual(["a1", "a2"]);
            expect(stabilizedB[0].subagents.map((s) => s.agent_id)).toEqual(["b1", "b2"]);
        } else {
            throw new Error("expected two name groups");
        }
    });
});

describe("groupCacheKey", () => {
    it("namespaces a WorkflowDispatch key with 'wf:' and a NameGroup key with 'name:<parentBlockId>:'", () => {
        const [wf] = buildDispatchChildren(
            [mkDispatch({ dispatch_id: "wf_1" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_1" })]
        ).filter(isWorkflowDispatch);
        const [ng] = buildDispatchChildren(
            [],
            [mk({ agent_id: "a1", display_name: "N" }), mk({ agent_id: "a2", display_name: "N" })]
        ).filter(isNameGroup);
        expect(groupCacheKey(wf)).toBe("wf:wf_1");
        expect(groupCacheKey(ng)).toBe("name:block-1:N");
    });
});

describe("pruneGroupIdentityCache", () => {
    it("drops entries for dispatches no longer live, keeps the rest", () => {
        const cache = new Map<string, WorkflowDispatch>();
        const groupA = buildDispatchChildren(
            [mkDispatch({ dispatch_id: "wf_a" })],
            [mk({ agent_id: "a1", dispatch_id: "wf_a" })]
        );
        const groupB = buildDispatchChildren(
            [mkDispatch({ dispatch_id: "wf_b" })],
            [mk({ agent_id: "b1", dispatch_id: "wf_b" })]
        );
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

    it("drops entries for name groups no longer live, keeps the rest", () => {
        const cache = new Map<string, WorkflowDispatch | NameGroup>();
        const groupA = buildDispatchChildren(
            [],
            [mk({ agent_id: "a1", display_name: "A" }), mk({ agent_id: "a2", display_name: "A" })]
        );
        const groupB = buildDispatchChildren(
            [],
            [mk({ agent_id: "b1", display_name: "B" }), mk({ agent_id: "b2", display_name: "B" })]
        );
        stabilizeGroupIdentity(cache, groupA);
        stabilizeGroupIdentity(cache, groupB);
        expect(cache.size).toBe(2);

        pruneGroupIdentityCache(cache, new Set(["name:block-1:A"]));
        expect(cache.size).toBe(1);
        expect(cache.has("name:block-1:A")).toBe(true);
        expect(cache.has("name:block-1:B")).toBe(false);
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
