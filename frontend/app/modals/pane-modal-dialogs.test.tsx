// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The dialogs moved from the window to the pane (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md §4
 * and §5): each opens inside the pane it is about, the sibling pane stays interactive, Escape closes only it,
 * and closing the pane gives the safe answer.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ModalLayer } from "@/app/element/ModalLayer";
import { __resetPaneRequests, openPaneModal } from "@/app/store/pane-requests";
import { PaneRequestModals } from "@/app/view/agent/pane-requests/PaneRequestDock";
import { ShutdownConfirm } from "@/app/view/agent/shutdown/shutdown-confirm";
import { ReplacePaneConfirm } from "./replace-pane-confirm";

vi.mock("@/app/notification/waiting-for-you", () => ({ startWaitingForYou: () => {}, endWaitingForYou: () => {} }));
// The panes that exist, by id: a test deletes one by setting it to undefined.
const panes = new Map<string, ReturnType<typeof createSignal<object | undefined>>>();
const pane = (id: string) => {
    if (!panes.has(id)) panes.set(id, createSignal<object | undefined>({ oid: id }));
    return panes.get(id)!;
};
vi.mock("@/app/store/global", () => ({
    MOS: { getMuxObjectAtom: (key: string) => pane(key.replace("block:", ""))[0] },
}));

/** Two panes side by side, each with the block frame's modal host and a button. */
function TwoPanes() {
    return (
        <>
            {["a", "b"].map((id) => (
                <div data-testid={`pane-${id}`}>
                    <ModalLayer scope="pane" fill>
                        <PaneRequestModals blockId={id} />
                        <button data-testid={`btn-${id}`}>content {id}</button>
                    </ModalLayer>
                </div>
            ))}
        </>
    );
}

afterEach(() => {
    cleanup();
    __resetPaneRequests();
    panes.clear();
});

const dialogs: { name: string; open: (blockId: string, onGone: () => void, answer: () => void) => void; title: string }[] = [
    {
        name: "Shut down agent",
        title: "Shut down Korp?",
        open: (blockId, onGone, answer) =>
            openPaneModal(blockId, ShutdownConfirm, { title: "Shut down Korp?", description: "x", onConfirm: answer }, { source: "shutdown", waits: false, onGone }),
    },
    {
        name: "Replace this pane",
        title: "Replace this pane with Shortcuts?",
        open: (blockId, onGone, answer) =>
            openPaneModal(blockId, ReplacePaneConfirm, { onConfirm: answer }, { source: "replace-pane", waits: false, onGone }),
    },
];

describe.each(dialogs)("$name, as a pane modal", ({ title, open }) => {
    it("covers only its pane: it sits in that pane's layer and the sibling pane's content stays live", () => {
        const { getByTestId } = render(() => <TwoPanes />);
        open("a", () => {}, () => {});
        const dialog = screen.getByText(title);
        expect(getByTestId("pane-a").contains(dialog)).toBe(true);
        expect(getByTestId("pane-b").contains(dialog)).toBe(false);
        // (jsdom has no `inert`, so the lock itself is covered by modal-region-lock; what shows here is the scope:
        // a dialog that fell back to the window would be in <body>, not in pane a.)
        expect(getByTestId("pane-b").querySelector(".modal-root")).toBeNull();
        expect(getByTestId("btn-b").hasAttribute("inert")).toBe(false);
    });

    it("Escape closes it and gives no answer", () => {
        const answer = vi.fn();
        render(() => <TwoPanes />);
        open("a", () => {}, answer);
        expect(screen.queryByText(title)).not.toBeNull();
        fireEvent.keyDown(document.querySelector(".modal-panel")!, { key: "Escape" });
        expect(screen.queryByText(title)).toBeNull();
        expect(answer).not.toHaveBeenCalled();
    });

    it("closing the pane while it is open takes it away and runs the safe answer", async () => {
        const gone = vi.fn();
        const answer = vi.fn();
        render(() => <TwoPanes />);
        open("a", gone, answer);
        pane("a")[1](undefined);
        await vi.waitFor(() => expect(gone).toHaveBeenCalledTimes(1));
        expect(answer).not.toHaveBeenCalled();
    });
});
