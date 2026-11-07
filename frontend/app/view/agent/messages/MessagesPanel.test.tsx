// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const waitingNow = vi.hoisted(() => ({ ids: [] as string[], reads: 0 }));
const dismissed = vi.fn();
vi.mock("../hooks/useJektsAwaitingYou", () => ({
    awaitingYou: (_blockId: string, nodes: readonly { id: string }[]) => {
        waitingNow.reads++;
        return nodes.filter((n) => waitingNow.ids.includes(n.id));
    },
    dismissAwaiting: (...a: unknown[]) => dismissed(...a),
}));

import type { DocumentNode, JektMessageNode } from "@/app/agent-model/node-types";
import { MessagesPanel, SEEN_AFTER_MS } from "./MessagesPanel";
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
    // Open: most of what is tested is the panel's content. The collapsed bar's tests say `open: false`.
    const [state, setState] = createSignal<MessagesState>({ ...DEFAULT_MESSAGES_STATE, enabled: true, open: true, ...initial });
    const [request, setRequest] = createSignal<{ nodeId: string; seq: number } | null>(null);
    let seq = 0;
    const controls: MessagesPanelControls = {
        state,
        patch: (c) => setState((s) => ({ ...s, ...c })),
        request,
        clearRequest: () => setRequest(null),
        showInPanel: (nodeId) => {
            setState((s) => ({ ...s, open: true }));
            setRequest({ nodeId, seq: ++seq });
        },
        hidden: () => false,
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
    vi.useRealTimers();
    vi.restoreAllMocks();
    scrolled.mockClear();
    cleanup();
    waitingNow.ids = [];
    waitingNow.reads = 0;
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

    it("is a bar labelled Jekt Messages with the count and a needs-you dot while collapsed, and opens from it", () => {
        const m = jekt({ tier: "sensitive", escalate: "required" });
        waitingNow.ids = [m.id];
        const { controls, view } = mount([m, jekt()], controlsFor({ open: false }));
        expect(view.container.querySelector(".messages-panel")).toBeNull();
        expect(view.container.querySelector(".messages-rail__text")?.textContent).toBe("Jekt Messages");
        expect(screen.getByText("2")).toBeTruthy();
        expect(view.container.querySelector(".messages-rail__needs")).toBeTruthy();
        fireEvent.click(screen.getByLabelText(/Jekt Messages, 2 unread, 1 need you/));
        expect(controls.state().open).toBe(true);
    });

    it("makes the whole bar one button: the label and the marks are inside it, so a click anywhere on it opens the panel", () => {
        const { controls, view } = mount([jekt()], controlsFor({ open: false }));
        const bar = view.container.querySelector(".messages-rail")!;
        const buttons = bar.querySelectorAll("button");
        expect(buttons).toHaveLength(1);
        expect(buttons[0].contains(bar.querySelector(".messages-rail__text"))).toBe(true);
        expect(buttons[0].contains(bar.querySelector(".messages-rail__marks"))).toBe(true);
        fireEvent.click(bar.querySelector(".messages-rail__text")!);
        expect(controls.state().open).toBe(true);
    });

    it("keeps the bar's column when the panel is open, empty and hidden from assistive technology, so the panel lies over the conversation", () => {
        const { view } = mount([jekt()]);
        const bar = view.container.querySelector(".messages-rail")!;
        expect(bar).toBeTruthy();
        expect(bar.getAttribute("aria-hidden")).toBe("true");
        expect(bar.querySelector("button")).toBeNull();
        const panel = view.container.querySelector(".messages-panel")!;
        expect(panel.classList.contains("ui-side-panel")).toBe(true);
        expect(panel.classList.contains("ui-side-panel--docked")).toBe(false);
    });

    it("closes from its close button, and on Esc, and puts the keyboard back on the bar", async () => {
        const { controls, view } = mount([jekt()]);
        fireEvent.click(screen.getByLabelText("Close"));
        expect(controls.state().open).toBe(false);
        expect(view.container.querySelector(".messages-panel")).toBeNull();
        controls.patch({ open: true });
        const panelButton = view.container.querySelector<HTMLElement>(".ui-side-panel__head button")!;
        await Promise.resolve();
        expect(document.activeElement).toBe(panelButton);
        fireEvent.keyDown(panelButton, { key: "Escape" });
        expect(controls.state().open).toBe(false);
        await Promise.resolve();
        expect(document.activeElement).toBe(view.container.querySelector(".messages-rail__button"));
    });

    it("opens without taking the keyboard from a text field (a search match in a held message opens it mid-typing)", async () => {
        const field = document.createElement("input");
        document.body.appendChild(field);
        field.focus();
        const { controls, view } = mount([jekt()], controlsFor({ open: false }));
        controls.showInPanel("j1");
        await Promise.resolve();
        expect(view.container.querySelector(".messages-panel")).toBeTruthy();
        expect(document.activeElement).toBe(field);
        field.remove();
    });

    it("does not offer Show in conversation for a message the conversation leaves out", () => {
        const held = jekt({ message: "held in the panel" });
        const c = controlsFor();
        const { view } = mount([held], { ...c, hidden: () => true });
        fireEvent.click(view.container.querySelector(".messages-row__head")!);
        expect(screen.getByText(/held in the panel/)).toBeTruthy();
        expect(screen.queryByText("Show in conversation")).toBeNull();
    });

    it("opens the message it was asked for (a search match inside it), opening the panel if it was collapsed", async () => {
        const a = jekt({ message: "first body text" });
        const b = jekt({ message: "second body text" });
        const { controls, view } = mount([a, b], controlsFor({ open: false }));
        expect(view.container.querySelector(".messages-panel")).toBeNull();
        controls.showInPanel(b.id);
        expect(openRows(view)).toBe(1);
        expect(view.container.querySelector(".messages-row__body")?.textContent).toContain("second body text");
        // The list is moved to the row once it is drawn (through the one scroll writer).
        await Promise.resolve();
        expect(scrolled).toHaveBeenCalled();
    });

    it("does none of its work while the pane's setting is off, and starts when it is turned on", () => {
        const controls = controlsFor({ enabled: false });
        mount([jekt(), jekt()], controls);
        expect(waitingNow.reads).toBe(0);
        controls.patch({ enabled: true });
        expect(waitingNow.reads).toBeGreaterThan(0);
    });

    it("does not replay a request when the panel is closed to the bar and opened again", () => {
        const a = jekt();
        const b = jekt();
        const { controls, view } = mount([a, b]);
        controls.showInPanel(b.id);
        expect(openRows(view)).toBe(1);
        controls.patch({ open: false });
        controls.patch({ open: true });
        expect(openRows(view)).toBe(0);
    });

    it("counts only arrivals as new, not the rows a filter brings back", () => {
        const old = jekt({ timestamp: 1_000 });
        const fresh = jekt({ timestamp: 50_000 });
        const { view } = mount([old, fresh], controlsFor({ seen: 20_000 }));
        const list = view.container.querySelector(".messages-panel__list") as HTMLElement;
        // The reader is up the list: 500 px of content in a 100 px window, at the top.
        Object.defineProperty(list, "scrollHeight", { value: 500, configurable: true });
        Object.defineProperty(list, "clientHeight", { value: 100, configurable: true });
        fireEvent.scroll(list);
        fireEvent.change(screen.getByLabelText("Show"), { target: { value: "unread" } });
        fireEvent.change(screen.getByLabelText("Show"), { target: { value: "all" } });
        expect(screen.queryByText(/ new$/)).toBeNull();
    });

    it("counts an arrival as new when a filter change left the list up from its tail, with no scroll event", async () => {
        const first = jekt({ from: "agent5", timestamp: 1_000 });
        const { setDoc, view } = mount([first, jekt({ from: "agent6", timestamp: 2_000 })]);
        const list = view.container.querySelector(".messages-panel__list") as HTMLElement;
        // The reader was at the tail; the filter then leaves a long list sitting at its top.
        Object.defineProperty(list, "scrollHeight", { value: 500, configurable: true });
        Object.defineProperty(list, "clientHeight", { value: 100, configurable: true });
        fireEvent.change(screen.getByLabelText("Show"), { target: { value: "all" } });
        fireEvent.change(screen.getByLabelText("Show"), { target: { value: "from:agent5" } });
        await Promise.resolve();
        setDoc([first, jekt({ from: "agent6", timestamp: 2_000 }), jekt({ from: "agent7", timestamp: 3_000 })]);
        await Promise.resolve();
        expect(screen.getByText("1 new")).toBeTruthy();
    });

    it("counts the moment as seen after the window gains focus, if it was unfocused when the time came", () => {
        vi.useFakeTimers();
        const focus = vi.spyOn(document, "hasFocus").mockReturnValue(false);
        const { controls } = mount([jekt(), jekt()]);
        vi.advanceTimersByTime(SEEN_AFTER_MS + 10);
        expect(controls.state().seen).toBe(0);
        focus.mockReturnValue(true);
        window.dispatchEvent(new Event("focus"));
        vi.advanceTimersByTime(SEEN_AFTER_MS + 10);
        expect(controls.state().seen).toBeGreaterThan(0);
    });

    it("follows new messages into the list", () => {
        const first = jekt({ from: "agent5" });
        const { setDoc, view } = mount([first]);
        setDoc([first, jekt({ from: "agent6" })]);
        expect(senders(view)).toEqual(["agent5", "agent6"]);
    });
});
