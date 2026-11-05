// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Closing a pane that still has busy agents, or a page that would ask "Leave site?", asks inside its tab
 * (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 10): the question sits in the tab's modal
 * layer, not the window, and each way out answers the close.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ModalLayer } from "@/app/element/ModalLayer";
import { TabCloseConfirms } from "./tab-close-confirms";

afterEach(cleanup);

const busy = { busy: [{ blockId: "b1", name: "Korp", turnActive: true, processCount: 2 }] };
const leave = { leaving: [{ blockId: "b2", site: "portal.example.com" }] };

function mount(opts: { busy?: typeof busy | null; leave?: typeof leave | null }) {
    const answerBusy = vi.fn();
    const answerLeave = vi.fn();
    const { container } = render(() => {
        const [b] = createSignal(opts.busy ?? null);
        const [l] = createSignal(opts.leave ?? null);
        return (
            <div data-testid="tab">
                <ModalLayer scope="tab">
                    <button>pane content</button>
                    <TabCloseConfirms busy={b} leave={l} answerBusy={answerBusy} answerLeave={answerLeave} />
                </ModalLayer>
            </div>
        );
    });
    return { container, answerBusy, answerLeave };
}

describe("TabCloseConfirms", () => {
    it("asks nothing when nothing is pending", () => {
        mount({});
        expect(screen.queryByText("Close this pane?")).toBeNull();
    });

    it("busy agents: named in the tab's layer; Shut down closes the pane", () => {
        const { answerBusy, container } = mount({ busy });
        expect(screen.getByTestId("tab").contains(screen.getByText("Close this pane?"))).toBe(true);
        expect(container.querySelector(".modal-layer-mount")?.contains(screen.getByText("Close this pane?"))).toBe(true);
        expect(screen.getByText("Korp — mid-turn, 2 processes running")).toBeTruthy();
        fireEvent.click(screen.getByText("Shut down"));
        expect(answerBusy).toHaveBeenCalledExactlyOnceWith(true);
    });

    it("busy agents: Cancel keeps the pane", () => {
        const { answerBusy } = mount({ busy });
        fireEvent.click(screen.getByText("Cancel"));
        expect(answerBusy).toHaveBeenCalledExactlyOnceWith(false);
    });

    it("a page that would ask to leave: names the site; Keep open keeps, Close anyway closes", () => {
        const kept = mount({ leave });
        expect(screen.getByText("portal.example.com may have unsaved changes.")).toBeTruthy();
        fireEvent.click(screen.getByText("Keep open"));
        expect(kept.answerLeave).toHaveBeenCalledExactlyOnceWith(false);
        cleanup();
        const closed = mount({ leave });
        fireEvent.click(screen.getByText("Close anyway"));
        expect(closed.answerLeave).toHaveBeenCalledExactlyOnceWith(true);
    });

    it("Escape keeps the pane open", () => {
        const { answerBusy } = mount({ busy });
        fireEvent.keyDown(document.querySelector(".modal-panel")!, { key: "Escape" });
        expect(answerBusy).toHaveBeenCalledExactlyOnceWith(false);
    });
});
