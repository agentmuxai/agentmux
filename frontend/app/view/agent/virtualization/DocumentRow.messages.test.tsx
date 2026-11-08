// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A jekt's row that the conversation still draws (the Messages panel leaves out only incoming ones it holds, before
 * the rows are drawn): it opens in place, as every message row does. The row, its disclosure rule are the real
 * ones; the container is a probe that records what it receives.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
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
import { DocumentRow } from "./DocumentRow";

const state = (over: Partial<DocumentState> = {}): DocumentState =>
    ({
        collapsedNodes: new Set(),
        pinnedNodes: new Set(),
        heldOpenNodes: new Set(),
        ...over,
    }) as unknown as DocumentState;

function mount(node: object, docState: DocumentState = state()) {
    const fns = { onToggleCollapse: vi.fn(), onTogglePin: vi.fn(), onHoldNodeOpen: vi.fn() };
    const [n] = createSignal(node as unknown as DocumentNode);
    const [s] = createSignal(docState);
    const [m] = createSignal(new Map<string, AgentDispatch>());
    render(() => <DocumentRow node={n} documentState={s} dispatchMatches={m} {...fns} />);
    return fns;
}

const jekt = { type: "jekt_message", id: "n1", direction: "incoming", timestamp: 1 };

afterEach(() => {
    cleanup();
    seen.props = undefined;
});

describe("a jekt's row in the conversation", () => {
    it("opens in place: its click and the e key flip the row", () => {
        const fns = mount(jekt, state({ pinnedNodes: new Set(["n1"]) }));
        expect(seen.props?.collapsed).toBe(false);
        seen.props?.onToggle();
        expect(fns.onTogglePin).toHaveBeenCalledWith("n1");
        fireEvent.keyDown(document.querySelector(".agent-document-row")!, { key: "e" });
        expect(fns.onTogglePin).toHaveBeenCalledTimes(2);
    });
});
