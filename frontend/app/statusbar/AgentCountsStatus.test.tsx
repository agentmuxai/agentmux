// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AttentionItem } from "@/app/notification/attention-list";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const open = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: open }));
vi.mock("@/app/store/agent-runtime-live", () => ({ sharedAgentRuntime: () => ({ rows: () => [] }) }));
const goTo = vi.hoisted(() => vi.fn());
vi.mock("@/app/notification/attention-go", () => ({ goToAttention: goTo, isParkedItem: (i: { blockId: string }) => i.blockId === "parked" }));
vi.mock("@/app/notification/attention-list", () => ({ attentionItems: () => [], attentionTint: (hex?: string) => hex }));

import { AgentCountsStatus } from "./AgentCountsStatus";

afterEach(() => {
    cleanup();
    open.mockClear();
    goTo.mockClear();
});

const item = (blockId: string, over: Partial<AttentionItem> = {}): AttentionItem => ({
    blockId,
    name: `Agent-${blockId}`,
    what: "asks you a question",
    since: Date.now() - 125_000,
    local: true,
    ...over,
});

const agent = (id: string, state: AgentRuntime["state"]): AgentRuntime => ({
    agent_id: id,
    state,
    desired: "running",
    name: id,
    provider: "claude",
    icon: "",
});

describe("AgentCountsStatus", () => {
    it("shows nothing while no agent runs", () => {
        const { container } = render(() => (
            <AgentCountsStatus rows={() => [agent("a", "stopped"), agent("b", "error")]} />
        ));
        expect(container.querySelector(".status-agent-counts")).toBeNull();
    });

    it("counts an agent held by a sign-in or a limit: it has a process", () => {
        render(() => <AgentCountsStatus rows={() => [agent("a", "signed_out"), agent("b", "limited")]} />);
        expect(screen.getByText("2 running")).toBeTruthy();
    });

    it("counts the running agents, and says how many wait on the user", () => {
        render(() => (
            <AgentCountsStatus
                rows={() => [agent("a", "working"), agent("b", "idle"), agent("c", "waiting_on_user")]}
            />
        ));
        expect(screen.getByText("3 running")).toBeTruthy();
        expect(screen.getByText("1 waiting on you")).toBeTruthy();
        expect(screen.getByRole("button").getAttribute("data-tip")).toBe(
            "3 agents are running, 1 waiting on you. Click to see who"
        );
    });

    it("leaves the waiting count out when nobody waits, and follows the rows", () => {
        const [rows, setRows] = createSignal([agent("a", "idle")]);
        render(() => <AgentCountsStatus rows={rows} />);
        expect(screen.getByText("1 running")).toBeTruthy();
        expect(screen.queryByText(/waiting on you/)).toBeNull();
        expect(screen.getByRole("button").getAttribute("data-tip")).toBe(
            "1 agent is running. Click to open the Swarm"
        );
        setRows([agent("a", "waiting_on_user")]);
        expect(screen.getByText("1 waiting on you")).toBeTruthy();
        setRows([]);
        expect(screen.queryByRole("button")).toBeNull();
    });

    it("opens the Swarm on a click", () => {
        render(() => <AgentCountsStatus rows={() => [agent("a", "working")]} />);
        fireEvent.click(screen.getByRole("button"));
        expect(open).toHaveBeenCalledWith("swarm");
    });

    // SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02.md §3.6.
    describe("the list of who waits", () => {
        it("stays while something waits, even when no agent has a process, and counts what only the list knows", () => {
            const { container } = render(() => <AgentCountsStatus rows={() => []} items={() => [item("b1")]} />);
            expect(container.querySelector(".status-agent-counts")).not.toBeNull();
            expect(screen.queryByText(/running/)).toBeNull();
            expect(screen.getByText("1 waiting on you")).toBeTruthy();
            cleanup();
            render(() => (
                <AgentCountsStatus rows={() => [agent("a", "working")]} items={() => [item("b1"), item("b2")]} />
            ));
            expect(screen.getByText("2 waiting on you")).toBeTruthy();
        });

        it("opens a row for each, oldest first, with how long and a button; the Swarm stays one click away", () => {
            render(() => (
                <AgentCountsStatus
                    rows={() => [agent("a", "waiting_on_user"), agent("b", "waiting_on_user")]}
                    items={() => [item("b1", { name: "Korp", what: "asks to approve a Bash call" }), item("b2", { local: false })]}
                />
            ));
            fireEvent.click(screen.getByRole("button", { name: /waiting on you/ }));
            expect(open).not.toHaveBeenCalled();
            expect(screen.getByRole("dialog", { name: "Agents waiting on you" })).toBeTruthy();
            expect(screen.getByText("Korp")).toBeTruthy();
            expect(screen.getByText("asks to approve a Bash call")).toBeTruthy();
            expect(screen.getByText("asks you a question (in another window)")).toBeTruthy();
            expect(screen.getAllByText(/2 min/)).toHaveLength(2);
            fireEvent.click(screen.getByRole("button", { name: "Open the Swarm" }));
            expect(open).toHaveBeenCalledWith("swarm");
            expect(screen.queryByRole("dialog")).toBeNull();
        });

        it("takes the person to the pane from its row and closes the list", () => {
            render(() => (
                <AgentCountsStatus rows={() => []} items={() => [item("b1", { name: "Korp" }), item("b2")]} />
            ));
            fireEvent.click(screen.getByRole("button", { name: /waiting on you/ }));
            fireEvent.click(screen.getByRole("button", { name: "Go to Korp" }));
            expect(goTo).toHaveBeenCalledWith(expect.objectContaining({ blockId: "b1" }));
            expect(screen.queryByRole("dialog")).toBeNull();
        });

        it("offers Open for an agent that runs with no pane", () => {
            render(() => <AgentCountsStatus rows={() => []} items={() => [item("parked", { name: "Clare" })]} />);
            fireEvent.click(screen.getByRole("button", { name: /waiting on you/ }));
            fireEvent.click(screen.getByRole("button", { name: "Open Clare" }));
            expect(goTo).toHaveBeenCalledWith(expect.objectContaining({ blockId: "parked" }));
        });

        it("closes the list when a narrow bar hides the entry", () => {
            // The popover's own positioning watches its anchor too: every observer is told.
            const notify: (() => void)[] = [];
            vi.stubGlobal(
                "ResizeObserver",
                class {
                    constructor(cb: () => void) {
                        notify.push(cb);
                    }
                    observe() {}
                    disconnect() {}
                }
            );
            render(() => <AgentCountsStatus rows={() => []} items={() => [item("b1")]} />);
            fireEvent.click(screen.getByRole("button", { name: /waiting on you/ }));
            expect(screen.getByRole("dialog")).toBeTruthy();
            // jsdom has no layout: an element with no boxes is one `display: none` took out of the bar.
            notify.forEach((cb) => cb());
            expect(screen.queryByRole("dialog")).toBeNull();
            vi.unstubAllGlobals();
        });

        it("closes the list when the last request is answered, and goes with the last agent", () => {
            const [items, setItems] = createSignal([item("b1")]);
            const [rows, setRows] = createSignal([agent("a", "waiting_on_user")]);
            render(() => <AgentCountsStatus rows={rows} items={items} />);
            fireEvent.click(screen.getByRole("button", { name: /waiting on you/ }));
            expect(screen.getByRole("dialog")).toBeTruthy();
            setItems([]);
            setRows([agent("a", "idle")]);
            expect(screen.queryByRole("dialog")).toBeNull();
            expect(screen.queryByText(/waiting on you/)).toBeNull();
            // The next request does not open the list unasked.
            setItems([item("b2")]);
            expect(screen.queryByRole("dialog")).toBeNull();
            expect(screen.getByRole("button", { name: /waiting on you/ }).getAttribute("aria-expanded")).toBe("false");
            setItems([]);
            setRows([]);
            expect(screen.queryByRole("button")).toBeNull();
            setItems([item("b3")]);
            expect(screen.queryByRole("dialog")).toBeNull();
        });
    });
});
