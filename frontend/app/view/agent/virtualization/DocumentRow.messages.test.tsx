// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An incoming jekt's row while the Messages panel is on: a closed one-line row whose click opens the message in the
 * panel, not a row that opens in place. The row, its disclosure rule and the context are the real ones; the
 * container is a probe that records what it receives.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const seen = vi.hoisted(() => ({ props: undefined as undefined | { collapsed: boolean; onToggle: () => void } }));
vi.mock("../components/OriginContainer", () => ({
    OriginContainer: (p: { collapsed: boolean; onToggle: () => void }) => {
        seen.props = p;
        return <div data-probe="OriginContainer" />;
    },
}));

import type { DocumentNode, DocumentState } from "../../../agent-model/types";
import type { AgentDispatch } from "../../swarm/swarm-model";
import { MessagesPanelContext, type MessagesPanelControls } from "../messages/messages-state";
import { DocumentRow } from "./DocumentRow";

const state = (over: Partial<DocumentState> = {}): DocumentState =>
    ({
        collapsedNodes: new Set(),
        pinnedNodes: new Set(),
        heldOpenNodes: new Set(),
        ...over,
    }) as unknown as DocumentState;

function mount(node: object, panel: MessagesPanelControls | undefined, docState: DocumentState = state()) {
    const fns = { onToggleCollapse: vi.fn(), onTogglePin: vi.fn(), onHoldNodeOpen: vi.fn() };
    const [n] = createSignal(node as unknown as DocumentNode);
    const [s] = createSignal(docState);
    const [m] = createSignal(new Map<string, AgentDispatch>());
    render(() => (
        <MessagesPanelContext.Provider value={panel}>
            <DocumentRow node={n} documentState={s} dispatchMatches={m} {...fns} />
        </MessagesPanelContext.Provider>
    ));
    return fns;
}

const jekt = { type: "jekt_message", id: "n1", direction: "incoming", timestamp: 1 };

const panel = (stubbed: boolean, showInPanel = vi.fn()): MessagesPanelControls =>
    ({ stubbed: () => stubbed, showInPanel }) as unknown as MessagesPanelControls;

afterEach(() => {
    cleanup();
    seen.props = undefined;
});

describe("an incoming jekt's row with the Messages panel", () => {
    it("is closed even when the row would be open, and its click opens the message in the panel", () => {
        const showInPanel = vi.fn();
        const fns = mount(jekt, panel(true, showInPanel), state({ pinnedNodes: new Set(["n1"]) }));
        expect(seen.props?.collapsed).toBe(true);
        seen.props?.onToggle();
        expect(showInPanel).toHaveBeenCalledWith("n1");
        expect(fns.onTogglePin).not.toHaveBeenCalled();
    });

    it("opens in place as before when the panel does not take it (off, outgoing or forced)", () => {
        const fns = mount(jekt, panel(false), state({ pinnedNodes: new Set(["n1"]) }));
        expect(seen.props?.collapsed).toBe(false);
        seen.props?.onToggle();
        expect(fns.onTogglePin).toHaveBeenCalledWith("n1");
    });

    it("opens in place with no panel in the pane's layout at all", () => {
        const fns = mount(jekt, undefined, state({ pinnedNodes: new Set(["n1"]) }));
        expect(seen.props?.collapsed).toBe(false);
        seen.props?.onToggle();
        expect(fns.onTogglePin).toHaveBeenCalledWith("n1");
    });
});
