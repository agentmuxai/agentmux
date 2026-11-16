// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

let openResult: () => Promise<string | undefined> = async () => undefined;
const openAgent = vi.fn(() => openResult());
vi.mock("@/app/store/agent-runtime-live", () => ({ openAgentInActiveTab: () => openAgent() }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: vi.fn() }));

import { OPEN_NOTE_MS, useAgentRowActivation } from "./swarm-agent-row-hooks";
import type { AgentTreeNode } from "./swarm-model";

const node = { blockId: "blk-1", agentName: "Korp" } as unknown as AgentTreeNode;
const parked = { "agent:parked": true, agentId: "a1" };
const shown = { agentId: "a1" };

/** A row's activation over a block meta the test changes, and what its note reads. */
function mountRow() {
    const [meta, setMeta] = createSignal<Record<string, unknown>>(parked);
    let dispose = () => {};
    const row = createRoot((d) => {
        dispose = d;
        return useAgentRowActivation(
            () => node,
            meta
        );
    });
    const click = async () => {
        row.onClick({ stopPropagation() {}, target: null, currentTarget: null } as unknown as MouseEvent);
        await vi.advanceTimersByTimeAsync(0);
    };
    return { row, setMeta, click, dispose };
}

describe("the note under a Swarm row after an open", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        openAgent.mockClear();
        openResult = async () => undefined;
    });
    afterEach(() => vi.useRealTimers());

    it("stays for a refusal on an agent that is still parked, until the next try", async () => {
        const m = mountRow();
        openResult = async () => "open in another pane";
        await m.click();
        expect(m.row.openRefused()).toBe("open in another pane");
        await vi.advanceTimersByTimeAsync(10 * OPEN_NOTE_MS);
        expect(m.row.openRefused()).toBe("open in another pane");
        // The next try clears it before it asks.
        openResult = async () => undefined;
        await m.click();
        expect(m.row.openRefused()).toBe("");
        m.dispose();
    });

    it("goes after a few seconds when the pane is already back, as the pane of 2026-11-15 was", async () => {
        const m = mountRow();
        // The attach un-parks the block before the reveal answers; the note comes after.
        openResult = async () => {
            m.setMeta(shown);
            return "it did not come to the front";
        };
        await m.click();
        expect(m.row.openRefused()).toBe("it did not come to the front");
        await vi.advanceTimersByTimeAsync(OPEN_NOTE_MS - 1);
        expect(m.row.openRefused()).toBe("it did not come to the front");
        await vi.advanceTimersByTimeAsync(2);
        expect(m.row.openRefused()).toBe("");
        m.dispose();
    });

    it("goes when the pane appears, though it was set while the agent was parked", async () => {
        const m = mountRow();
        openResult = async () => "provider can't run here";
        await m.click();
        expect(m.row.openRefused()).toBe("provider can't run here");
        m.setMeta(shown);
        await vi.advanceTimersByTimeAsync(0);
        expect(m.row.openRefused()).toBe("");
        m.dispose();
    });

    it("does not leave a timer behind when the row goes away", async () => {
        const m = mountRow();
        openResult = async () => {
            m.setMeta(shown);
            return "it did not come to the front";
        };
        await m.click();
        m.dispose();
        expect(vi.getTimerCount()).toBe(0);
    });
});
