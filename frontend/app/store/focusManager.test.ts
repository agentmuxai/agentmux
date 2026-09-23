// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md: `requestNodeFocus()` used to be
// a hardcoded no-op even though every within-tab pane-selection path
// (click, arrow-key nav, Cmd+1..9, pane creation) already dispatched through
// it via the layout tree's FocusNode/InsertNode/MagnifyNodeToggle reducer
// cases. This suite pins the fixed behavior — real delegation to
// refocusNode() — plus the new claimFocusOnMount() entry point views use on
// their own mount, and the modal guard both share.

import { beforeEach, describe, expect, it, vi } from "vitest";

const hasOpenModals = vi.fn(() => false);
vi.mock("@/app/store/modalmodel", () => ({
    modalsModel: { hasOpenModals: () => hasOpenModals() },
}));

let focusedNode: { id: string; data?: { blockId?: string } } | null = null;
const focusNode = vi.fn();
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({
        focusedNode: () => focusedNode,
        focusNode: (id: string) => focusNode(id),
    }),
}));

const giveFocus = vi.fn(() => true);
let bcmForBlockId: Record<string, { viewModel: { giveFocus: () => boolean } }> = {};
vi.mock("@/app/store/global", () => ({
    getBlockComponentModel: (blockId: string) => bcmForBlockId[blockId],
}));

vi.mock("@/util/focusutil", () => ({
    focusedBlockId: () => focusedNode?.data?.blockId ?? null,
}));

import { focusManager } from "./focusManager";

describe("focusManager", () => {
    beforeEach(() => {
        hasOpenModals.mockReset().mockReturnValue(false);
        focusNode.mockClear();
        giveFocus.mockClear().mockReturnValue(true);
        focusedNode = { id: "node-1", data: { blockId: "block-1" } };
        bcmForBlockId = { "block-1": { viewModel: { giveFocus } } };

        // jsdom doesn't ship a real dummy-focus target; keep the fallback
        // path callable without a DOM query throwing.
        vi.spyOn(document, "getElementById").mockReturnValue(null);
    });

    describe("requestNodeFocus", () => {
        it("delegates to refocusNode() instead of being a no-op", () => {
            focusManager.requestNodeFocus();
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });
    });

    describe("refocusNode", () => {
        it("focuses the active tab's currently-focused block", () => {
            focusManager.refocusNode();
            expect(focusNode).toHaveBeenCalledWith("node-1");
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });

        it("falls back to the dummy-focus element when giveFocus() fails", () => {
            giveFocus.mockReturnValue(false);
            const dummy = { focus: vi.fn() };
            vi.mocked(document.getElementById).mockReturnValue(dummy as unknown as HTMLElement);

            focusManager.refocusNode();

            expect(document.getElementById).toHaveBeenCalledWith("block-1-dummy-focus");
            expect(dummy.focus).toHaveBeenCalledTimes(1);
        });

        it("does nothing when no node is focused", () => {
            focusedNode = null;
            focusManager.refocusNode();
            expect(giveFocus).not.toHaveBeenCalled();
        });

        it("never steals the caret from an open modal", () => {
            hasOpenModals.mockReturnValue(true);
            focusManager.refocusNode();
            expect(focusNode).not.toHaveBeenCalled();
            expect(giveFocus).not.toHaveBeenCalled();
        });
    });

    describe("claimFocusOnMount", () => {
        it("calls giveFocus when the mounting block is the active tab's focused node", () => {
            const localGiveFocus = vi.fn(() => true);
            focusManager.claimFocusOnMount("block-1", localGiveFocus);
            expect(localGiveFocus).toHaveBeenCalledTimes(1);
        });

        it("does NOT call giveFocus for a block in a background tab or an unfocused pane", () => {
            // The mounting pane's own blockId doesn't match the ACTIVE tab's
            // focused node — e.g. a background-tab pane created via muxsh,
            // per SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md §5 constraint 1.
            const localGiveFocus = vi.fn(() => true);
            focusManager.claimFocusOnMount("some-other-block", localGiveFocus);
            expect(localGiveFocus).not.toHaveBeenCalled();
        });

        it("does NOT call giveFocus while a modal is open, even for the focused block", () => {
            hasOpenModals.mockReturnValue(true);
            const localGiveFocus = vi.fn(() => true);
            focusManager.claimFocusOnMount("block-1", localGiveFocus);
            expect(localGiveFocus).not.toHaveBeenCalled();
        });
    });
});
