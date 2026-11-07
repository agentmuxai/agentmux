// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Closing a PANE that still has busy agents, or a page that would ask "Leave site?", asks inside that pane
 * only (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 10, widened by the operator on
 * 2026-11-07: a modal's reach matches the scope of its issue). The question sits in the layer of the pane being
 * closed, every other pane and the tab stay live, and each way out answers the close.
 */

import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ModalLayer } from "@/app/element/ModalLayer";
import { __resetPaneRequests } from "@/app/store/pane-requests";
import { PaneRequestModals } from "@/app/view/agent/pane-requests/PaneRequestDock";
import { askInPane, CloseBusyConfirm, LeaveSiteConfirm } from "./pane-close-confirms";

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
// The layout: which block each pane shows. "a" is a stack of a-1 (background) and a-2 (shown).
vi.mock("@/layout/lib/layoutModelHooks", () => ({
    getLayoutModelForTabById: () => ({
        getNodeByBlockId: (id: string) =>
            id.startsWith("a-") ? { data: { blockId: "a-2", activeBlockId: "a-2", blockStack: ["a-1", "a-2"] } } : { data: { blockId: id } },
    }),
}));

const busy = [{ blockId: "a-2", name: "Korp", turnActive: true, processCount: 2 }];
const leaving = [{ blockId: "w", site: "portal.example.com" }];

/** Two panes and a tab bar button, each pane with the block frame's modal host. */
function Window(props: { shown: string[] }) {
    return (
        <div data-testid="window">
            <button data-testid="tab-bar">tab bar</button>
            {props.shown.map((id) => (
                <div data-testid={`pane-${id}`}>
                    <ModalLayer scope="pane" fill>
                        <PaneRequestModals blockId={id} />
                        <button data-testid={`btn-${id}`}>content {id}</button>
                    </ModalLayer>
                </div>
            ))}
        </div>
    );
}

afterEach(() => {
    cleanup();
    __resetPaneRequests();
    panes.clear();
});

describe("closing a pane asks in that pane only", () => {
    it("busy agents: the question is in the pane's layer, not the tab's, and the rest stays live", async () => {
        const { getByTestId } = render(() => <Window shown={["a-2", "b"]} />);
        const answer = askInPane("t1", "a-1", "close-busy", CloseBusyConfirm, { busy });
        const dialog = screen.getByText("Close this pane?");
        // Asked for a background tab of the stack, drawn in the pane that is on screen.
        expect(getByTestId("pane-a-2").contains(dialog)).toBe(true);
        expect(getByTestId("pane-b").contains(dialog)).toBe(false);
        expect(getByTestId("pane-b").querySelector(".modal-root")).toBeNull();
        expect(getByTestId("btn-b").hasAttribute("inert")).toBe(false);
        expect(getByTestId("tab-bar").hasAttribute("inert")).toBe(false);
        expect(screen.getByText("Korp — mid-turn, 2 processes running")).toBeTruthy();
        fireEvent.click(screen.getByText("Shut down"));
        await expect(answer).resolves.toBe(true);
        expect(screen.queryByText("Close this pane?")).toBeNull();
    });

    it("busy agents: Cancel and Escape keep the pane", async () => {
        render(() => <Window shown={["a-2"]} />);
        const cancelled = askInPane("t1", "a-2", "close-busy", CloseBusyConfirm, { busy });
        fireEvent.click(screen.getByText("Cancel"));
        await expect(cancelled).resolves.toBe(false);
        const escaped = askInPane("t1", "a-2", "close-busy", CloseBusyConfirm, { busy });
        fireEvent.keyDown(document.querySelector(".modal-panel")!, { key: "Escape" });
        await expect(escaped).resolves.toBe(false);
    });

    it("a page that would ask to leave: names the site; Keep open keeps, Close anyway closes", async () => {
        render(() => <Window shown={["w"]} />);
        const kept = askInPane("t1", "w", "close-leave", LeaveSiteConfirm, { leaving });
        expect(screen.getByText("portal.example.com may have unsaved changes.")).toBeTruthy();
        fireEvent.click(screen.getByText("Keep open"));
        await expect(kept).resolves.toBe(false);
        const closed = askInPane("t1", "w", "close-leave", LeaveSiteConfirm, { leaving });
        fireEvent.click(screen.getByText("Close anyway"));
        await expect(closed).resolves.toBe(true);
    });

    it("a second close of the same pane shares the first question's answer", async () => {
        render(() => <Window shown={["a-2"]} />);
        const first = askInPane("t1", "a-2", "close-busy", CloseBusyConfirm, { busy });
        const second = askInPane("t1", "a-1", "close-busy", CloseBusyConfirm, { busy });
        expect(second).toBe(first);
        expect(screen.getAllByText("Close this pane?")).toHaveLength(1);
        fireEvent.click(screen.getByText("Shut down"));
        await expect(Promise.all([first, second])).resolves.toEqual([true, true]);
    });

    it("two panes close at once: each asks in its own pane, and each answer is its own", async () => {
        const { getByTestId } = render(() => <Window shown={["a-2", "b"]} />);
        const forA = askInPane("t1", "a-2", "close-busy", CloseBusyConfirm, { busy });
        const forB = askInPane("t1", "b", "close-busy", CloseBusyConfirm, { busy });
        expect(getByTestId("pane-a-2").textContent).toContain("Close this pane?");
        expect(getByTestId("pane-b").textContent).toContain("Close this pane?");
        fireEvent.click(within(getByTestId("pane-b")).getByText("Shut down"));
        await expect(forB).resolves.toBe(true);
        expect(getByTestId("pane-a-2").textContent).toContain("Close this pane?");
        fireEvent.click(within(getByTestId("pane-a-2")).getByText("Cancel"));
        await expect(forA).resolves.toBe(false);
    });

    it("the pane going away first answers no", async () => {
        render(() => <Window shown={["b"]} />);
        const answer = askInPane("t1", "b", "close-busy", CloseBusyConfirm, { busy });
        pane("b")[1](undefined);
        await expect(answer).resolves.toBe(false);
    });
});
