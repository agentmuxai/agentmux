// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const waitingNow = vi.hoisted(() => ({ ids: [] as string[] }));
const dismissed = vi.fn();
vi.mock("../hooks/useJektsAwaitingYou", () => ({
    awaitingYou: (_blockId: string, nodes: readonly { id: string }[]) =>
        nodes.filter((n) => waitingNow.ids.includes(n.id)),
    dismissAwaiting: (...a: unknown[]) => dismissed(...a),
}));

import type { DocumentNode, JektMessageNode } from "@/app/agent-model/node-types";
import { MessagesPanel } from "./MessagesPanel";
import { DEFAULT_MESSAGES_STATE, type MessagesPanelControls, type MessagesState } from "./messages-state";

let n = 0;
const jekt = (over: Partial<JektMessageNode> = {}): JektMessageNode => ({
    type: "jekt_message",
    id: `j${++n}`,
    from: "agent2",
    to: "agent1",
    message: `message number ${n}`,
    raw: "",
    tier: "coord",
    deliveryTier: "host",
    trust: "host-verified",
    msgId: `m${n}`,
    priority: "normal",
    direction: "incoming",
    timestamp: 10_000 + n,
    ...over,
});

/** The pane's controls over plain signals: what the block meta would hold. */
function controlsFor(initial: Partial<MessagesState> = {}) {
    const [state, setState] = createSignal<MessagesState>({ ...DEFAULT_MESSAGES_STATE, enabled: true, ...initial });
    const [request, setRequest] = createSignal<{ nodeId: string; seq: number } | null>(null);
    let seq = 0;
    const controls: MessagesPanelControls = {
        state,
        patch: (c) => setState((s) => ({ ...s, ...c })),
        request,
        showInPanel: (nodeId) => setRequest({ nodeId, seq: ++seq }),
        stubbed: () => false,
    };
    return controls;
}

function mount(nodes: DocumentNode[], controls = controlsFor(), jumpTo = vi.fn()) {
    const [doc, setDoc] = createSignal<readonly DocumentNode[]>(nodes);
    const view = render(() => <MessagesPanel blockId="p1" controls={controls} nodes={doc} jumpTo={jumpTo} zoom={1} />);
    return { view, controls, jumpTo, setDoc };
}

/** The senders of the rows in the list, top to bottom (the filter's options also name them). */
const senders = (view: { container: HTMLElement }): string[] =>
    Array.from(view.container.querySelectorAll(".messages-row__from")).map((e) => e.textContent ?? "");
const openRows = (view: { container: HTMLElement }): number =>
    view.container.querySelectorAll(".messages-row__body").length;

// jsdom has no Element.scrollTo: the panel's smooth moves go through it.
const scrolled = vi.fn();
beforeEach(() => {
    HTMLElement.prototype.scrollTo = scrolled as never;
});

afterEach(() => {
    scrolled.mockClear();
    cleanup();
    waitingNow.ids = [];
    dismissed.mockClear();
});

describe("MessagesPanel", () => {
    it("draws nothing while the pane's setting is off", () => {
        const { view } = mount([jekt()], controlsFor({ enabled: false }));
        expect(view.container.querySelector(".messages-dock")).toBeNull();
    });

    it("lists the incoming messages with who sent them, and leaves the agent's own sends out", () => {
        const { view } = mount([
            jekt({ from: "agent7" }),
            jekt({ direction: "outgoing", to: "agent9", from: "agent1" }),
            jekt({ from: "agent8" }),
        ]);
        expect(senders(view)).toEqual(["agent7", "agent8"]);
    });

    it("opens a message to its body and metadata, and Show in conversation jumps to it", () => {
        const m = jekt({ message: "please look at #42" });
        const { jumpTo, view } = mount([m]);
        expect(screen.queryByText("Show in conversation")).toBeNull();
        fireEvent.click(view.container.querySelector(".messages-row__head")!);
        expect(screen.getByText(/please look at #42/)).toBeTruthy();
        expect(screen.getByText(new RegExp(`MSGID: ${m.msgId}`))).toBeTruthy();
        fireEvent.click(screen.getByText("Show in conversation"));
        expect(jumpTo).toHaveBeenCalledWith(m.id);
    });

    it("shows a count of what has not been seen, and Mark all read clears it", () => {
        const { controls } = mount([jekt(), jekt(), jekt({ replayed: true })]);
        expect(screen.getByText("2")).toBeTruthy();
        fireEvent.click(screen.getByText("Mark all read"));
        expect(controls.state().seen).toBeGreaterThan(0);
        expect(screen.queryByText("2")).toBeNull();
    });

    it("filters to the unread", () => {
        const old = jekt({ from: "agent5", timestamp: 1_000 });
        const fresh = jekt({ from: "agent6", timestamp: 50_000 });
        const { view } = mount([old, fresh], controlsFor({ seen: 20_000 }));
        expect(senders(view)).toEqual(["agent5", "agent6"]);
        fireEvent.change(screen.getByLabelText("Show"), { target: { value: "unread" } });
        expect(senders(view)).toEqual(["agent6"]);
    });

    it("keeps a message that needs the person above the list with the card's two actions", () => {
        const m = jekt({ tier: "sensitive", escalate: "required", trust: "network-claimed" });
        waitingNow.ids = [m.id];
        const { jumpTo } = mount([m]);
        const strip = screen.getByLabelText("Needs you");
        expect(strip.textContent).toContain("agent2 sent a sensitive message");
        fireEvent.click(screen.getByText("Show message"));
        expect(jumpTo).toHaveBeenCalledWith(m.id);
        fireEvent.click(screen.getByText("Dismiss"));
        expect(dismissed).toHaveBeenCalledWith("p1", [m]);
    });

    it("is a rail with the count and a needs-you dot while closed, and opens from it", () => {
        const m = jekt({ tier: "sensitive", escalate: "required" });
        waitingNow.ids = [m.id];
        const { controls, view } = mount([m, jekt()], controlsFor({ open: false }));
        expect(view.container.querySelector(".messages-panel")).toBeNull();
        expect(screen.getByText("2")).toBeTruthy();
        expect(view.container.querySelector(".messages-rail__needs")).toBeTruthy();
        fireEvent.click(screen.getByLabelText(/Messages, 2 unread, 1 need you/));
        expect(controls.state().open).toBe(true);
    });

    it("opens the message a stub in the conversation asked for", async () => {
        const a = jekt({ message: "first body text" });
        const b = jekt({ message: "second body text" });
        const { controls, view } = mount([a, b]);
        expect(openRows(view)).toBe(0);
        controls.showInPanel(b.id);
        expect(openRows(view)).toBe(1);
        expect(view.container.querySelector(".messages-row__body")?.textContent).toContain("second body text");
        // The list is moved to the row once it is drawn (through the one scroll writer).
        await Promise.resolve();
        expect(scrolled).toHaveBeenCalled();
    });

    it("follows new messages into the list", () => {
        const first = jekt({ from: "agent5" });
        const { setDoc, view } = mount([first]);
        setDoc([first, jekt({ from: "agent6" })]);
        expect(senders(view)).toEqual(["agent5", "agent6"]);
    });
});
