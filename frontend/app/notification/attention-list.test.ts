// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { describe, expect, it, vi } from "vitest";
import { mergeAttention } from "./attention-list";

vi.mock("@/app/store/agent-runtime-live", () => ({ sharedAgentRuntime: () => ({ rows: () => [] }) }));
vi.mock("./waiting-for-you", () => ({ waitingPanes: () => [] }));
vi.mock("@/app/block/pane-color-scheme", () => ({ paneRoleColor: () => undefined }));
vi.mock("@/app/block/pane-identity", () => ({ isLightThemeActive: () => false }));
vi.mock("@/app/store/global", () => ({ MOS: {}, workspace: () => null }));
vi.mock("@/app/element/pane-tab-model", () => ({ describePaneTab: () => ({ label: "" }) }));

const row = (over: Partial<AgentRuntime>): AgentRuntime =>
    ({
        agent_id: "a",
        state: "idle",
        desired: "running",
        name: "Agent",
        provider: "claude",
        icon: "",
        ...over,
    }) as AgentRuntime;
const pane = (blockId: string, what: string, since: number) => ({ blockId, what, since });
const seen = () => 500;

// SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02.md §3.1.
describe("mergeAttention", () => {
    it("names a pane of this window by its agent's row, in its colour, with the registry's words", () => {
        const items = mergeAttention(
            [pane("b1", "asks to approve a Bash call", 100)],
            [row({ name: "Korp", color: "#3b82f6", block_id: "b1" })],
            seen
        );
        expect(items).toEqual([
            {
                blockId: "b1",
                name: "Korp",
                agentId: "a",
                color: "#3b82f6",
                what: "asks to approve a Bash call",
                since: 100,
                local: true,
            },
        ]);
    });

    it("names a pane that is not an agent's by its own name, and says it is a pane", () => {
        const paneName = (blockId: string) => (blockId === "remotes" ? "Connectors · Remotes" : undefined);
        const items = mergeAttention(
            [pane("remotes", "asks you to confirm", 100), pane("b1", "asks you a question", 200), pane("new", "needs you", 300)],
            [row({ name: "Korp", block_id: "b1" })],
            seen,
            () => undefined,
            paneName
        );
        expect(items.map((i) => [i.blockId, i.name, i.pane])).toEqual([
            ["remotes", "Connectors · Remotes", true],
            // An agent's row names it, whatever the pane is called.
            ["b1", "Korp", undefined],
            // Neither a row nor a name of its own: the old words.
            ["new", "An agent", undefined],
        ]);
    });

    it("adds an agent that only srv says waits that this window knows is elsewhere, once, marked as not local", () => {
        const rows = [row({ name: "Clare", color: "#aa0000", block_id: "b2", state: "waiting_on_user" })];
        const items = mergeAttention([pane("b1", "asks you a question", 100)], rows, seen, () => false);
        expect(items.map((i) => [i.blockId, i.local])).toEqual([
            ["b1", true],
            ["b2", false],
        ]);
        expect(items[1]).toMatchObject({ name: "Clare", what: "is waiting for you", since: 500 });
    });

    it("says a pane is in another window only when this window knows it is not here", () => {
        const rows = [
            row({ block_id: "b4", state: "waiting_on_user" }),
            row({ block_id: "b5", state: "waiting_on_user" }),
        ];
        const here = (b: string) => (b === "b4" ? true : b === "b5" ? false : undefined);
        const items = mergeAttention([], rows, seen, here);
        expect(items.map((i) => [i.blockId, i.local])).toEqual([
            ["b4", true],
            ["b5", false],
        ]);
        // A tab not loaded yet cannot place it: it is not called elsewhere.
        expect(mergeAttention([], rows.slice(0, 1), seen, () => undefined)[0].local).toBe(true);
    });

    it("counts a pane that both feeds name once, from the registry", () => {
        const rows = [row({ name: "Korp", block_id: "b1", state: "waiting_on_user" })];
        expect(mergeAttention([pane("b1", "asks you a question", 100)], rows, seen)).toHaveLength(1);
    });

    it("skips an agent that waits without a pane, and one that does not wait", () => {
        const rows = [
            row({ block_id: undefined, state: "waiting_on_user" }),
            row({ block_id: "b3", state: "working" }),
        ];
        expect(mergeAttention([], rows, seen)).toEqual([]);
    });

    it("falls back to 'An agent' when srv has no row for the pane, and lists the oldest first", () => {
        const items = mergeAttention([pane("late", "x", 300), pane("early", "y", 100)], [], seen);
        expect(items.map((i) => [i.blockId, i.name])).toEqual([
            ["early", "An agent"],
            ["late", "An agent"],
        ]);
    });
});
