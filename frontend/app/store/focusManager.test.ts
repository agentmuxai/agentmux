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

// Which block (if any) currently holds a real user caret — the
// SPEC_PANE_CLICK_THROUGH_INPUT_FOCUS_2026_09_23.md guard. focusutil.test.ts
// covers the DOM logic itself; here it's just a switch.
let caretInBlock: string | null = null;
vi.mock("@/util/focusutil", () => ({
    focusedBlockId: () => focusedNode?.data?.blockId ?? null,
    userCaretInBlock: (blockId: string) => caretInBlock === blockId,
}));

import { focusManager } from "./focusManager";

describe("focusManager", () => {
    beforeEach(() => {
        hasOpenModals.mockReset().mockReturnValue(false);
        focusNode.mockClear();
        giveFocus.mockClear().mockReturnValue(true);
        focusedNode = { id: "node-1", data: { blockId: "block-1" } };
        caretInBlock = null;
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

        // SPEC_PANE_CLICK_THROUGH_INPUT_FOCUS_2026_09_23.md: the click that
        // SELECTS a pane must not take the caret from the input it landed in.
        it("selects the node but keeps the caret when the user already put it in an input in that block", () => {
            caretInBlock = "block-1";
            const dummy = { focus: vi.fn() };
            vi.mocked(document.getElementById).mockReturnValue(dummy as unknown as HTMLElement);

            focusManager.refocusNode();

            expect(focusNode).toHaveBeenCalledWith("node-1");
            expect(giveFocus).not.toHaveBeenCalled();
            expect(dummy.focus).not.toHaveBeenCalled();
        });

        it("still moves the caret when it's in a DIFFERENT block (keyboard nav, #3519)", () => {
            caretInBlock = "block-2";
            focusManager.refocusNode();
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });

        it("focuses the dummy fallback without scrolling", () => {
            giveFocus.mockReturnValue(false);
            const dummy = { focus: vi.fn() };
            vi.mocked(document.getElementById).mockReturnValue(dummy as unknown as HTMLElement);

            focusManager.refocusNode();

            expect(dummy.focus).toHaveBeenCalledWith({ preventScroll: true });
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

        it("does NOT call giveFocus when the user's caret is already in an input in that block", () => {
            caretInBlock = "block-1";
            const localGiveFocus = vi.fn(() => true);
            focusManager.claimFocusOnMount("block-1", localGiveFocus);
            expect(localGiveFocus).not.toHaveBeenCalled();
        });
    });
});
