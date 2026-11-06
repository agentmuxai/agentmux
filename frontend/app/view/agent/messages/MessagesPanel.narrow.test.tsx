// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Messages panel in a narrow pane: no column and no rail, a button in the pane's header, and the panel over the
 * conversation while that button has it open.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../hooks/useJektsAwaitingYou", () => ({
    awaitingYou: () => [],
    dismissAwaiting: vi.fn(),
}));

import type { DocumentNode, JektMessageNode } from "@/app/agent-model/node-types";
import { DeclIconButton } from "@/app/element/ui/DeclIconButton";
import { MessagesPanel } from "./MessagesPanel";
import { createNarrow, NARROW_BELOW_PX } from "./messages-overlay";
import {
    DEFAULT_MESSAGES_STATE,
    messagesHeaderButtons,
    type MessagesPanelControls,
    type MessagesState,
} from "./messages-state";

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

function controlsFor(initial: Partial<MessagesState> = {}) {
    const [state, setState] = createSignal<MessagesState>({ ...DEFAULT_MESSAGES_STATE, enabled: true, ...initial });
    const [request, setRequest] = createSignal<{ nodeId: string; seq: number } | null>(null);
    const [overlayOpen, setOverlayOpen] = createSignal(false);
    const [narrow, setNarrow] = createSignal(false);
    let seq = 0;
    const controls: MessagesPanelControls = {
        state,
        patch: (c) => setState((s) => ({ ...s, ...c })),
        request,
        clearRequest: () => setRequest(null),
        showInPanel: (nodeId) => {
            if (narrow()) setOverlayOpen(true);
            setRequest({ nodeId, seq: ++seq });
        },
        stubbed: () => false,
        overlayOpen,
        setOverlayOpen,
        narrow,
        setNarrow,
    };
    return controls;
}

/** The pane's width as the dock's parent reports it. */
function paneWidth(px: number) {
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(px);
}

function mount(nodes: DocumentNode[], controls = controlsFor(), zoom = 1) {
    const [doc] = createSignal<readonly DocumentNode[]>(nodes);
    const view = render(() => (
        <MessagesPanel blockId="p9" controls={controls} nodes={doc} jumpTo={vi.fn()} zoom={zoom} />
    ));
    return { view, controls };
}

beforeEach(() => {
    HTMLElement.prototype.scrollTo = vi.fn() as never;
});
afterEach(() => {
    vi.restoreAllMocks();
    cleanup();
});

describe("createNarrow", () => {
    const narrowAt = (width: number, zoom = 1) => {
        const el = document.createElement("div");
        vi.spyOn(el, "clientWidth", "get").mockReturnValue(width);
        let result = false;
        const dispose = createRoot((d) => {
            // onMount runs when the root has finished.
            const narrow = createNarrow(
                () => el,
                () => zoom
            );
            queueMicrotask(() => (result = narrow()));
            return d;
        });
        return { dispose, read: () => Promise.resolve().then(() => result) };
    };

    it("is narrow below the threshold, wide at and above it, and wide while the width is unknown", async () => {
        expect(await narrowAt(NARROW_BELOW_PX - 1).read()).toBe(true);
        expect(await narrowAt(NARROW_BELOW_PX).read()).toBe(false);
        expect(await narrowAt(0).read()).toBe(false);
    });

    it("counts the pane's zoom: the room the conversation has is the width over it", async () => {
        expect(await narrowAt(800, 1.5).read()).toBe(true); // 533 px of room
        expect(await narrowAt(800, 1).read()).toBe(false);
    });
});

describe("MessagesPanel in a narrow pane", () => {
    it("shows no column and no rail, and puts a Messages button in the header", () => {
        paneWidth(400);
        const { view } = mount([jekt(), jekt()]);
        expect(view.container.querySelector(".messages-dock--idle")).not.toBeNull();
        expect(view.container.querySelector(".messages-panel")).toBeNull();
        expect(view.container.querySelector(".messages-rail")).toBeNull();
        const buttons = messagesHeaderButtons("p9");
        expect(buttons).toHaveLength(1);
        expect(buttons[0].title).toBe("Messages, 2 unread");
    });

    it("draws the button's face in every header that shows it, not only the last one", () => {
        paneWidth(400);
        mount([jekt()]);
        const decl = messagesHeaderButtons("p9")[0];
        const a = render(() => <DeclIconButton decl={decl} />);
        const b = render(() => <DeclIconButton decl={decl} />);
        expect(a.container.querySelector(".messages-header-button")).not.toBeNull();
        expect(b.container.querySelector(".messages-header-button")).not.toBeNull();
    });

    it("shuts the open overlay on Esc pressed inside it, and leaves Esc alone otherwise", () => {
        paneWidth(400);
        const { view, controls } = mount([jekt()]);
        controls.setOverlayOpen(true);
        const outside = vi.fn();
        document.addEventListener("keydown", outside);
        fireEvent.keyDown(view.container.querySelector(".messages-panel__list")!, { key: "Escape" });
        document.removeEventListener("keydown", outside);
        expect(controls.overlayOpen()).toBe(false);
        expect(outside).not.toHaveBeenCalled();
        fireEvent.keyDown(view.container.querySelector(".messages-dock")!, { key: "Escape" });
        expect(controls.overlayOpen()).toBe(false);
    });

    it("opens the panel over the conversation from the button, and closes it from the panel's close button", () => {
        paneWidth(400);
        const { view, controls } = mount([jekt()]);
        messagesHeaderButtons("p9")[0].click?.(new MouseEvent("click"));
        expect(controls.overlayOpen()).toBe(true);
        expect(view.container.querySelector(".messages-dock--overlay:not(.messages-dock--idle)")).not.toBeNull();
        expect(screen.getByText("Messages")).toBeTruthy();
        fireEvent.click(screen.getByLabelText("Close"));
        expect(controls.overlayOpen()).toBe(false);
        expect(view.container.querySelector(".messages-panel")).toBeNull();
        // The wide-pane state is left alone: closing the overlay is not closing the docked panel.
        expect(controls.state().open).toBe(true);
    });

    it("opens for a stub's request, on that message", () => {
        paneWidth(400);
        const a = jekt();
        const b = jekt();
        const { view, controls } = mount([a, b]);
        controls.showInPanel(b.id);
        expect(controls.overlayOpen()).toBe(true);
        expect(view.container.querySelectorAll(".messages-row__body")).toHaveLength(1);
    });

    it("shuts the overlay when the pane grows wide, so narrowing it again does not open it by itself", () => {
        paneWidth(400);
        const { controls } = mount([jekt()]);
        controls.setOverlayOpen(true);
        paneWidth(900);
        cleanup();
        const wide = mount([jekt()], controls);
        expect(controls.narrow()).toBe(false);
        expect(controls.overlayOpen()).toBe(false);
        wide.view.unmount();
    });

    it("takes the header button away when the pane is wide, and when the panel is off or unmounted", () => {
        paneWidth(900);
        const { view } = mount([jekt()]);
        expect(messagesHeaderButtons("p9")).toHaveLength(0);
        expect(view.container.querySelector(".messages-dock--overlay")).toBeNull();
        expect(view.container.querySelector(".messages-panel")).not.toBeNull();
        cleanup();
        paneWidth(400);
        const off = mount([jekt()], controlsFor({ enabled: false }));
        expect(messagesHeaderButtons("p9")).toHaveLength(0);
        off.view.unmount();
        const on = mount([jekt()]);
        expect(messagesHeaderButtons("p9")).toHaveLength(1);
        on.view.unmount();
        expect(messagesHeaderButtons("p9")).toHaveLength(0);
    });
});
