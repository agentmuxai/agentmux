// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm status chip shows three states: working (red, mid-turn), question
 * (theme primary, mid-turn and waiting on the user, with the question timer's
 * countdown) and idle (green, everything else). User requests, 2026-09-27 and
 * 2026-10-02.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
// The question panel publishes its timer to block meta; nothing to talk to here.
vi.mock("@/app/store/services", () => ({ ObjectService: { UpdateObjectMeta: () => Promise.resolve() } }));

import { AgentStatusChip, chipStatus, type AgentDisplayStatus } from "./swarm-view";
import {
    noteQuestionActivity,
    resetQuestionTimersForTests,
    setQuestionTimerDormant,
    startQuestionTimer,
} from "@/app/store/question-timer";
import { AgentQuestionPanel } from "@/app/view/agent/components/AgentQuestionPanel";
import type { ToolNode } from "@/app/view/agent/types";

afterEach(() => {
    cleanup();
    resetQuestionTimersForTests();
});

describe("chipStatus", () => {
    it("mid-turn states are working", () => {
        for (const s of ["working", "tools", "stopping"] as AgentDisplayStatus[]) {
            expect(chipStatus(s)).toBe("working");
        }
    });

    it("everything else is idle", () => {
        for (const s of ["idle", "error", "disconnected", "unknown", "interrupted"] as AgentDisplayStatus[]) {
            expect(chipStatus(s)).toBe("idle");
        }
    });

    it("mid-turn and waiting on the user is question", () => {
        for (const s of ["working", "tools", "stopping"] as AgentDisplayStatus[]) {
            expect(chipStatus(s, true)).toBe("question");
        }
    });

    it("a waiting flag on an agent with no turn in flight stays idle", () => {
        expect(chipStatus("idle", true)).toBe("idle");
    });
});

describe("AgentStatusChip: question", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    const chip = (container: HTMLElement) => container.querySelector(".swarm-status-chip")!;
    const countdown = (container: HTMLElement) => chip(container).querySelector(".swarm-status-countdown");

    it("is labelled question, with the question class and a pulsing dot", () => {
        const { container } = render(() => <AgentStatusChip status="working" awaitingUser />);
        expect(chip(container).textContent).toBe("question");
        expect(chip(container).classList.contains("swarm-status-chip--question")).toBe(true);
        expect(chip(container).querySelector(".swarm-status-dot--question")).not.toBeNull();
    });

    it("shows the question timer's seconds and band, ticking", () => {
        startQuestionTimer("b1", { durationMs: 30_000, onExpire: vi.fn() });
        const { container } = render(() => <AgentStatusChip status="working" awaitingUser blockId="b1" />);
        expect(countdown(container)!.textContent).toBe("30s");
        expect(countdown(container)!.classList.contains("swarm-status-countdown--default")).toBe(true);

        vi.advanceTimersByTime(20_000);
        expect(countdown(container)!.textContent).toBe("10s");
        expect(countdown(container)!.classList.contains("swarm-status-countdown--warning")).toBe(true);
        vi.advanceTimersByTime(5_000);
        expect(countdown(container)!.classList.contains("swarm-status-countdown--critical")).toBe(true);
    });

    it("reads paused while the user is active", () => {
        startQuestionTimer("b1", { durationMs: 30_000, onExpire: vi.fn() });
        const { container } = render(() => <AgentStatusChip status="working" awaitingUser blockId="b1" />);
        noteQuestionActivity("b1");
        expect(countdown(container)!.textContent).toBe("· paused");
    });

    it("reads paused while the pane tab is in the background", () => {
        startQuestionTimer("b1", { durationMs: 30_000, onExpire: vi.fn() });
        setQuestionTimerDormant("b1", true);
        const { container } = render(() => <AgentStatusChip status="working" awaitingUser blockId="b1" />);
        expect(countdown(container)!.textContent).toBe("· paused");
    });

    it("shows plain question without timer state", () => {
        const { container } = render(() => <AgentStatusChip status="working" awaitingUser blockId="nobody" />);
        expect(chip(container).textContent).toBe("question");
        expect(countdown(container)).toBeNull();
    });

    it("shows no countdown unless it's in the question state", () => {
        startQuestionTimer("b1", { durationMs: 30_000, onExpire: vi.fn() });
        const { container } = render(() => <AgentStatusChip status="idle" awaitingUser blockId="b1" />);
        expect(chip(container).textContent).toBe("idle");
    });

    // One timer surfaced in two places: the panel and the chip never disagree.
    it("shows the same seconds as the question panel", () => {
        const question: ToolNode = {
            type: "tool",
            id: "q1",
            tool: "Other",
            params: {},
            status: "awaiting_answer",
            collapsed: false,
            summary: "",
            question: {
                type: "ask_user_question",
                tool_use_id: "q1",
                questions: [{ question: "Pick", header: "H", multiSelect: false, options: [{ label: "A" }] }],
            },
        };
        const [pending] = createSignal<ToolNode[]>([question]);
        const { container } = render(() => (
            <>
                <AgentQuestionPanel blockId="b1" pending={pending} onAnswer={vi.fn()} onCancel={vi.fn()} />
                <AgentStatusChip status="working" awaitingUser blockId="b1" />
            </>
        ));
        const panelSeconds = () => /in (\d+)s/.exec(container.querySelector(".agent-question-panel-countdown")!.textContent!)![1];
        const chipSeconds = () => countdown(container)!.textContent!.replace("s", "");
        expect(panelSeconds()).toBe("30");
        expect(chipSeconds()).toBe("30");
        vi.advanceTimersByTime(7_000);
        expect(panelSeconds()).toBe("23");
        expect(chipSeconds()).toBe("23");
    });
});

describe("AgentStatusChip", () => {
    it("labels and classes the chip with one of the two states; 'tools' never appears", () => {
        const { container } = render(() => <AgentStatusChip status="tools" />);
        const chip = container.querySelector(".swarm-status-chip")!;
        expect(chip.textContent).toBe("working");
        expect(chip.classList.contains("swarm-status-chip--working")).toBe(true);
        expect(chip.querySelector(".swarm-status-dot--working")).not.toBeNull();
    });

    it("follows the status as it changes (props read reactively, not destructured once)", () => {
        const [status, setStatus] = createSignal<AgentDisplayStatus>("working");
        const { container } = render(() => <AgentStatusChip status={status()} />);
        const chip = () => container.querySelector(".swarm-status-chip")!;
        expect(chip().textContent).toBe("working");
        setStatus("idle");
        expect(chip().textContent).toBe("idle");
        expect(chip().classList.contains("swarm-status-chip--idle")).toBe(true);
        setStatus("tools");
        expect(chip().textContent).toBe("working");
    });
});

// The selection border follows the focused pane's ACTIVE tab: switching a
// multi-tab pane to another agent keeps the same focused node, so reading the
// node alone left the border stuck on the previous agent (user, 2026-09-27).
describe("focusedActiveBlockId", () => {
    it("follows a tab switch inside the same focused pane", async () => {
        const { createRoot, createMemo } = await import("solid-js");
        const { focusedActiveBlockId } = await import("./swarm-view");
        const [tree, setTree] = createSignal(0);
        const node = { data: { blockId: "agentA", activeBlockId: "agentA", blockStack: ["agentA", "agentB"] } };
        const model = {
            localTreeStateAtom: () => tree(),
            // Same node object before and after the switch, like LayoutModel's memo.
            focusedNode: () => node,
        } as any;
        createRoot((dispose) => {
            const focused = createMemo(() => focusedActiveBlockId(model));
            expect(focused()).toBe("agentA");
            node.data.activeBlockId = "agentB";
            node.data.blockId = "agentB";
            setTree(1); // setActiveBlockInStack republishes the tree state
            expect(focused()).toBe("agentB");
            dispose();
        });
    });
});
