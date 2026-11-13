// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm lists an agent once. A second pane of the agent, closed at startup because the agent is open in
 * another pane, got a row of its own: the same name twice, the second with an old status (#47409).
 */
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { describe, expect, it } from "vitest";
import { closedSecondPanes, paneIdentity } from "./swarm-duplicate-panes";
import { agentsWithoutRow } from "./swarm-runtime";

const closedFor = (holder: string) => ({ "agent:last_failure": { code: "open_in_other_pane", holderBlockId: holder } });
const hidden = (blocks: Record<string, Record<string, unknown> | undefined>) =>
    [...closedSecondPanes(Object.entries(blocks).map(([id, meta]) => paneIdentity(id, meta)))].sort();

describe("paneIdentity", () => {
    it("reads the agent from the instance the block is bound to, else its id", () => {
        expect(paneIdentity("b", { agentId: "template", agentInstanceId: "uid-1" }).agent).toBe("uid-1");
        expect(paneIdentity("b", { agentId: "uid-2" }).agent).toBe("uid-2");
        expect(paneIdentity("b", undefined)).toEqual({ blockId: "b", agent: "", closedForAnotherPane: false, holderBlockId: "" });
    });

    it("marks a pane closed for another pane, and no other failure", () => {
        expect(paneIdentity("b", { agentId: "a", ...closedFor("live") })).toMatchObject({
            closedForAnotherPane: true,
            holderBlockId: "live",
        });
        const signedOut = paneIdentity("b", { agentId: "a", "agent:last_failure": { code: "signed_out", holderBlockId: "x" } });
        expect(signedOut).toMatchObject({ closedForAnotherPane: false, holderBlockId: "" });
    });
});

describe("closedSecondPanes", () => {
    it("drops the closed second pane of an agent whose live pane is listed", () => {
        expect(
            hidden({
                "masty-live": { agentName: "Masty", agentId: "m", agentInstanceId: "m" },
                "masty-old": { agentName: "Masty", agentId: "m", agentInstanceId: "m", ...closedFor("masty-live") },
                "clare-live": { agentName: "Clare", agentId: "c", agentInstanceId: "c" },
                "clare-old": { agentName: "Clare", agentId: "c", agentInstanceId: "c", ...closedFor("clare-live") },
                maop: { agentName: "Maop", agentId: "p", "agent:parked": true },
            })
        ).toEqual(["clare-old", "masty-old"]);
    });

    it("drops it when the pane it names is gone but another pane of the agent is listed", () => {
        expect(
            hidden({
                live: { agentId: "m" },
                old: { agentInstanceId: "m", agentId: "m", ...closedFor("a-block-from-before-the-restart") },
            })
        ).toEqual(["old"]);
    });

    it("keeps it when the pane it names has another agent now: it is its own agent's only row", () => {
        expect(hidden({ reused: { agentId: "someone-else" }, old: { agentId: "m", ...closedFor("reused") } })).toEqual([]);
        expect(hidden({ picker: {}, old: { agentId: "m", ...closedFor("picker") } })).toEqual([]);
    });

    it("drops a closed pane that names no agent for the pane it names", () => {
        expect(hidden({ live: { agentId: "m" }, old: { ...closedFor("live") } })).toEqual(["old"]);
    });

    it("keeps a closed pane that is the agent's only row", () => {
        expect(hidden({ old: { agentId: "m", ...closedFor("gone") }, other: { agentId: "x" } })).toEqual([]);
    });

    it("keeps both rows when both panes are closed for each other", () => {
        expect(hidden({ a: { agentId: "m", ...closedFor("b") }, b: { agentId: "m", ...closedFor("a") } })).toEqual([]);
    });

    it("keeps two agents launched from one template, and two blocks that name no agent", () => {
        expect(
            hidden({
                one: { agentId: "template", agentInstanceId: "uid-1" },
                two: { agentId: "template", agentInstanceId: "uid-2" },
                bare: {},
                bare2: { ...closedFor("nowhere") },
            })
        ).toEqual([]);
    });
});

// srv's record of a running agent follows its newest pane and stays on it after srv closes that pane for the
// older one. With the closed pane's row gone, the agent was listed a second time under "Running with no pane".
describe("an agent whose record names its closed second pane", () => {
    it("has a row, so it is not also listed as running with no pane", () => {
        const closed = closedSecondPanes([
            paneIdentity("live", { agentId: "a" }),
            paneIdentity("old", { agentId: "a", ...closedFor("live") }),
        ]);
        const record: AgentRuntime = { agent_id: "a", state: "idle", desired: "running", name: "A", provider: "claude", icon: "", block_id: "old" };
        expect(agentsWithoutRow([record], new Set(["live", ...closed]))).toEqual([]);
        expect(agentsWithoutRow([record], new Set(["live"]))).toEqual([record]);
    });
});
