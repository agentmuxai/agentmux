// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The working mark on a Swarm row keeps moving while the row's node is replaced again and again (#47754: "all
 * the agents in the swarm have a stuck animation for their working status", with the Swarm rebuilding its tree
 * 10 to 136 times a minute). The mark is one character on the shared ticker, so what is pinned here is that a
 * rebuild neither replaces its element nor stops the ticker, and that the frame still steps on every tick.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { DockNodeStatusCommand: () => Promise.resolve() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
vi.mock("@/app/store/agent-runtime-live", () => ({ openAgentInActiveTab: async () => undefined }));

import { SPINNER_FRAME_MS, SPINNER_FRAMES } from "@/app/element/AsciiSpinner";
import { __resetAllSlots, registerPane } from "@/app/store/agent-document-store";
import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import { AgentRow } from "./swarm-view";

const BLOCK = "wa1";

/** A working agent's node; each call is a new object with the same content, as every rebuild makes. */
function workingNode(over: Partial<AgentTreeNode> = {}): AgentTreeNode {
    return {
        blockId: BLOCK,
        agentName: "Korp",
        agentProvider: "claude",
        activitySummary: null,
        line: { text: "No activity yet", source: "status" },
        contextTokens: null,
        agentStatus: "running",
        agentToolRows: [],
        workflowRows: [],
        shellRows: [],
        cronRows: [],
        todoRows: [],
        todosTruncated: 0,
        todosPartial: false,
        currentTool: null,
        backgroundTasks: NO_BACKGROUND_TASKS,
        ...over,
    } as AgentTreeNode;
}

const model = {
    isAgentCollapsed: () => true,
    toggleAgentCollapsed: () => {},
    isSelected: () => false,
    toggleSelected: () => {},
    countdownStateAtom: () => new Map(),
    isExpanded: () => false,
    getDispatchDetail: () => undefined,
    toggleDispatchExpanded: () => {},
    pauseCountdown: () => {},
    resumeCountdown: () => {},
    retireRow: () => {},
} as unknown as SwarmViewModel;

describe("the working mark on a Swarm row, across rebuilds of the tree", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => {
        cleanup();
        __resetAllSlots();
        vi.useRealTimers();
    });

    function mountRows(count: number) {
        registerPane(BLOCK);
        const [node, setNode] = createSignal(workingNode());
        const view = render(() => (
            <div class="swarm-view" tabIndex={-1}>
                {Array.from({ length: count }, (_, i) => (
                    <AgentRow node={node} focusedBlockId={() => null} model={model} rowKey={`row-${i}`} />
                ))}
            </div>
        ));
        const marks = () => [...view.container.querySelectorAll<HTMLElement>(".swarm-status-spinner")];
        return { view, setNode, marks };
    }

    it("shows the mark on a working row", () => {
        const { marks } = mountRows(1);
        expect(marks()).toHaveLength(1);
        expect(SPINNER_FRAMES as readonly string[]).toContain(marks()[0].textContent);
    });

    it("steps every tick although the node is replaced between ticks, and keeps the same element", () => {
        const { setNode, marks } = mountRows(3);
        const first = marks();
        const seen: string[] = [];
        for (let i = 0; i < 8; i++) {
            vi.advanceTimersByTime(SPINNER_FRAME_MS / 2);
            setNode(workingNode());
            vi.advanceTimersByTime(SPINNER_FRAME_MS / 2);
            setNode(workingNode());
            seen.push(marks()[0].textContent ?? "");
        }
        // One frame per tick, in order, over two full turns of the spinner.
        const start = SPINNER_FRAMES.indexOf(seen[0] as (typeof SPINNER_FRAMES)[number]);
        expect(start).toBeGreaterThanOrEqual(0);
        expect(seen).toEqual(seen.map((_, i) => SPINNER_FRAMES[(start + i) % SPINNER_FRAMES.length]));
        // Not replaced: the same three elements, still attached.
        expect(marks()).toHaveLength(3);
        marks().forEach((m, i) => expect(m).toBe(first[i]));
    });

    it("keeps the same timers running while rows update faster than a tick, and the frame still steps", () => {
        const { setNode, marks } = mountRows(4);
        // The rows' own clocks and the one ticker; a rebuild must neither add nor drop any.
        const timers = vi.getTimerCount();
        for (let i = 0; i < 40; i++) {
            vi.advanceTimersByTime(SPINNER_FRAME_MS / 4);
            setNode(workingNode());
        }
        expect(vi.getTimerCount()).toBe(timers);
        const a = marks()[0].textContent;
        vi.advanceTimersByTime(SPINNER_FRAME_MS);
        expect(marks()[0].textContent).not.toBe(a);
    });
});
