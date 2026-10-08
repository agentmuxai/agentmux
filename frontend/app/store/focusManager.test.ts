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

import { createSignal } from "solid-js";
import { beforeEach, describe, expect, it, vi } from "vitest";

const hasOpenModals = vi.fn(() => false);
vi.mock("@/app/store/modalmodel", () => ({
    modalsModel: { hasOpenModals: () => hasOpenModals() },
}));

let focusedNode: { id: string; data?: { blockId?: string } } | null = null;
const focusNode = vi.fn();
const activeLayoutModel = {
    focusedNode: () => focusedNode,
    focusNode: (id: string) => focusNode(id),
};
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => activeLayoutModel,
}));

const giveFocus = vi.fn(() => true);
let bcmForBlockId: Record<string, { viewModel: { giveFocus?: () => boolean } }> = {};
const [activeTabId, setActiveTabId] = createSignal("tab-1");
const [tabSwitching, setTabSwitching] = createSignal(false);
const [gatingNodeIds, setGatingNodeIds] = createSignal<ReadonlySet<string>>(new Set());
vi.mock("@/app/store/tab-reveal", () => ({
    tabSwitching: () => tabSwitching(),
    gatingNodeIds: () => gatingNodeIds(),
}));
vi.mock("@/app/store/global", () => ({
    atoms: { activeTabId: () => activeTabId() },
    getBlockComponentModel: (blockId: string) => bcmForBlockId[blockId],
}));

// Which block (if any) currently holds a real user caret — the
// SPEC_PANE_CLICK_THROUGH_INPUT_FOCUS_2026_09_23.md guard. focusutil.test.ts
// covers the DOM logic itself; here it's just a switch.
let caretInBlock: string | null = null;
let caretOutsidePanes = false;
vi.mock("@/util/focusutil", () => ({
    focusedBlockId: () => focusedNode?.data?.blockId ?? null,
    userCaretInBlock: (blockId: string) => caretInBlock === blockId,
    caretInEditableOutsidePanes: () => caretOutsidePanes,
}));

// Frames run only when a test says so.
let frames: FrameRequestCallback[] = [];
vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
vi.stubGlobal("cancelAnimationFrame", () => {
    frames = [];
});
function runFrames(n = 1): void {
    for (let i = 0; i < n; i++) {
        const due = frames;
        frames = [];
        due.forEach((cb) => cb(0));
    }
}

import { push as pushModal, remove as removeModal } from "@/app/element/modal-stack";
import { FOCUS_RETRY_FRAMES, focusManager, installFocusFollowsSelection } from "./focusManager";

describe("focusManager", () => {
    beforeEach(() => {
        hasOpenModals.mockReset().mockReturnValue(false);
        focusNode.mockClear();
        giveFocus.mockClear().mockReturnValue(true);
        focusedNode = { id: "node-1", data: { blockId: "block-1" } };
        caretInBlock = null;
        caretOutsidePanes = false;
        frames = [];
        document.body.innerHTML = "";
        vi.spyOn(document, "hasFocus").mockReturnValue(true);
        bcmForBlockId = { "block-1": { viewModel: { giveFocus } } };

        // jsdom doesn't ship a real dummy-focus target; keep the fallback
        // path callable without a DOM query throwing. Other ids resolve
        // normally (jsdom's querySelector("#id") goes through here too).
        const byId = Document.prototype.getElementById;
        vi.spyOn(document, "getElementById").mockImplementation((id) => (id.endsWith("-dummy-focus") ? null : byId.call(document, id)));
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

    // SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md R1: one reconciler behind
    // every "the selection changed without the user placing the caret" path.
    describe("ensureSelectionFocused", () => {
        it("puts the caret in the active tab's selected pane", () => {
            focusManager.ensureSelectionFocused("selection", activeLayoutModel);
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });

        it("ignores a selection change in a background tab's layout", () => {
            focusManager.ensureSelectionFocused("selection", { other: true });
            expect(giveFocus).not.toHaveBeenCalled();
        });

        it("leaves the caret where the user put it outside the panes (tab rename, a search box)", () => {
            caretOutsidePanes = true;
            focusManager.ensureSelectionFocused("selection", activeLayoutModel);
            expect(giveFocus).not.toHaveBeenCalled();
        });

        it("never runs under an open modal", () => {
            hasOpenModals.mockReturnValue(true);
            focusManager.ensureSelectionFocused("selection");
            expect(giveFocus).not.toHaveBeenCalled();
        });

        // The window is in the background (`exit`, then the user switched
        // apps): a browser pane's giveFocus() would take native focus.
        it("never runs while the window doesn't have focus", () => {
            vi.mocked(document.hasFocus).mockReturnValue(false);
            focusManager.ensureSelectionFocused("selection");
            expect(giveFocus).not.toHaveBeenCalled();
        });

        it("stops retrying when the window loses focus", () => {
            giveFocus.mockReturnValue(false);
            focusManager.ensureSelectionFocused("selection");
            vi.mocked(document.hasFocus).mockReturnValue(false);
            runFrames(3);
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });

        // A declarative <Modal> (the pane-close confirmation) is only on the
        // modal stack, not in modalsModel.
        it("never runs under a modal-stack modal covering the pane", () => {
            document.body.innerHTML = `<div data-blockid="block-1"></div>`;
            pushModal({ id: "confirm", scope: "window", lockEl: document.body, close: () => {} });
            try {
                focusManager.ensureSelectionFocused("selection");
                expect(giveFocus).not.toHaveBeenCalled();
            } finally {
                removeModal("confirm");
            }
        });

        it("still runs when the only modal is a pane modal in another pane", () => {
            document.body.innerHTML = `<div data-blockid="block-1"></div><div data-blockid="block-2"><div id="lock"></div></div>`;
            pushModal({ id: "pane", scope: "pane", lockEl: document.getElementById("lock")!, close: () => {} });
            try {
                focusManager.ensureSelectionFocused("selection");
                expect(giveFocus).toHaveBeenCalledTimes(1);
            } finally {
                removeModal("pane");
            }
        });

        it("stops retrying once a modal opens over the pane", () => {
            giveFocus.mockReturnValue(false);
            document.body.innerHTML = `<div data-blockid="block-1"></div>`;
            focusManager.ensureSelectionFocused("selection");
            pushModal({ id: "confirm", scope: "window", lockEl: document.body, close: () => {} });
            try {
                runFrames(3);
                expect(giveFocus).toHaveBeenCalledTimes(1);
            } finally {
                removeModal("confirm");
            }
        });

        it("falls back to the pane's data-pane-focus input when its view has no giveFocus", () => {
            bcmForBlockId = { "block-1": { viewModel: {} } };
            document.body.innerHTML = `<div data-blockid="block-1"><input id="filter" data-pane-focus /></div>`;
            const input = document.body.querySelector<HTMLInputElement>("#filter")!;
            // jsdom has no layout, so nothing is "visible" without this.
            input.checkVisibility = () => true;

            focusManager.ensureSelectionFocused("selection");

            expect(document.activeElement).toBe(input);
        });

        it("selects what the data-pane-focus input still holds, so typing replaces it", () => {
            bcmForBlockId = { "block-1": { viewModel: {} } };
            document.body.innerHTML = `<div data-blockid="block-1"><input id="filter" data-pane-focus value="old query" /></div>`;
            const input = document.body.querySelector<HTMLInputElement>("#filter")!;
            input.checkVisibility = () => true;

            focusManager.ensureSelectionFocused("selection");

            expect([input.selectionStart, input.selectionEnd]).toEqual([0, "old query".length]);
        });

        it("skips a disabled or hidden data-pane-focus input", () => {
            bcmForBlockId = { "block-1": { viewModel: {} } };
            document.body.innerHTML = `<div data-blockid="block-1"><input id="a" data-pane-focus disabled /><input id="b" data-pane-focus /><input id="c" data-pane-focus /></div>`;
            const [a, b, c] = ["a", "b", "c"].map((id) => document.getElementById(id) as HTMLInputElement);
            a.checkVisibility = () => true;
            b.checkVisibility = () => false;
            c.checkVisibility = () => true;

            focusManager.ensureSelectionFocused("selection");

            expect(document.activeElement).toBe(c);
        });

        it("retries a view that is still mounting until it takes the caret", () => {
            giveFocus.mockReturnValueOnce(false).mockReturnValueOnce(false).mockReturnValue(true);
            focusManager.ensureSelectionFocused("selection");
            expect(giveFocus).toHaveBeenCalledTimes(1);
            runFrames(2);
            expect(giveFocus).toHaveBeenCalledTimes(3);
            runFrames(5);
            expect(giveFocus).toHaveBeenCalledTimes(3);
        });

        it("gives up after FOCUS_RETRY_FRAMES", () => {
            giveFocus.mockReturnValue(false);
            focusManager.ensureSelectionFocused("selection");
            runFrames(FOCUS_RETRY_FRAMES + 10);
            expect(giveFocus).toHaveBeenCalledTimes(1 + FOCUS_RETRY_FRAMES);
        });

        it("stops retrying once the selection moves on", () => {
            giveFocus.mockReturnValue(false);
            focusManager.ensureSelectionFocused("selection");
            focusedNode = { id: "node-2", data: { blockId: "block-2" } };
            runFrames(3);
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });

        // Cmd+, from a terminal: Settings is selected but not mounted yet, and
        // the caret is still in the terminal the selection left.
        it("keeps retrying while the caret is still in the pane the selection left", () => {
            giveFocus.mockReturnValueOnce(false).mockReturnValueOnce(false).mockReturnValue(true);
            document.body.innerHTML = `<div data-blockid="term"><textarea id="xterm"></textarea></div><div data-blockid="block-1"></div>`;
            (document.getElementById("xterm") as HTMLTextAreaElement).focus();
            focusManager.ensureSelectionFocused("selection");
            runFrames(2);
            expect(giveFocus).toHaveBeenCalledTimes(3);
        });

        // The hamburger menu opened Settings: the caret is on its button.
        it("keeps retrying while the caret is on a window-chrome button", () => {
            giveFocus.mockReturnValueOnce(false).mockReturnValueOnce(false).mockReturnValue(true);
            document.body.innerHTML = `<button id="menu">≡</button><div data-blockid="block-1"></div>`;
            (document.getElementById("menu") as HTMLButtonElement).focus();
            focusManager.ensureSelectionFocused("selection");
            runFrames(2);
            expect(giveFocus).toHaveBeenCalledTimes(3);
        });

        it("stops retrying once the user puts the caret somewhere", () => {
            giveFocus.mockReturnValue(false);
            focusManager.ensureSelectionFocused("selection");
            caretOutsidePanes = true; // a text field outside the panes
            document.body.innerHTML = `<input id="elsewhere" />`;
            document.body.querySelector<HTMLInputElement>("#elsewhere")!.focus();
            runFrames(3);
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });
    });

    // R2: the safety net, and a window tab becoming active.
    describe("installFocusFollowsSelection", () => {
        beforeEach(() => {
            installFocusFollowsSelection();
            vi.spyOn(console, "info").mockImplementation(() => {});
        });

        it("returns a caret that fell from a pane to <body> to the selection", () => {
            document.body.innerHTML = `<div data-blockid="block-9"><textarea id="t"></textarea></div>`;
            const t = document.body.querySelector<HTMLTextAreaElement>("#t")!;
            t.focus();
            // A pane closing: Chromium fires focusout as the element goes,
            // and the caret lands on <body>.
            t.blur();
            runFrames(1);
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });

        // A press on agent output blurs the composer to <body>; refocusing it
        // would collapse the selection the user is dragging.
        it("leaves it alone when the user pressed inside the pane", () => {
            document.body.innerHTML = `<div data-blockid="block-9"><p id="out">text</p><textarea id="t"></textarea></div>`;
            const t = document.body.querySelector<HTMLTextAreaElement>("#t")!;
            t.focus();
            document.getElementById("out")?.dispatchEvent(new Event("pointerdown", { bubbles: true }));
            t.blur();
            runFrames(1);
            expect(giveFocus).not.toHaveBeenCalled();
        });

        it("gives the selection the caret when the window comes back with it on <body>", () => {
            (document.activeElement as HTMLElement | null)?.blur();
            window.dispatchEvent(new Event("focus"));
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });

        it("leaves a caret the user placed alone when the window comes back", () => {
            document.body.innerHTML = `<input id="x" />`;
            document.body.querySelector<HTMLInputElement>("#x")!.focus();
            window.dispatchEvent(new Event("focus"));
            expect(giveFocus).not.toHaveBeenCalled();
        });

        it("leaves it alone when the press that moved it was outside the panes", () => {
            document.body.innerHTML = `<div id="chrome"></div><div data-blockid="block-9"><textarea id="t"></textarea></div>`;
            const t = document.body.querySelector<HTMLTextAreaElement>("#t")!;
            t.focus();
            document.getElementById("chrome")?.dispatchEvent(new Event("pointerdown", { bubbles: true }));
            t.blur();
            runFrames(1);
            expect(giveFocus).not.toHaveBeenCalled();
        });

        it("leaves it alone when the whole window lost focus", () => {
            vi.mocked(document.hasFocus).mockReturnValue(false);
            document.body.innerHTML = `<div data-blockid="block-9"><textarea id="t"></textarea></div>`;
            const t = document.body.querySelector<HTMLTextAreaElement>("#t")!;
            t.focus();
            t.blur();
            runFrames(1);
            expect(giveFocus).not.toHaveBeenCalled();
        });

        it("ignores focus leaving something outside the panes", () => {
            document.body.innerHTML = `<input id="rename" />`;
            const r = document.body.querySelector<HTMLInputElement>("#rename")!;
            r.focus();
            r.blur();
            runFrames(1);
            expect(giveFocus).not.toHaveBeenCalled();
        });

        // A view under a hidden reveal gate says it took focus but can't
        // (#4479); when the gate lifts, try again.
        it("focuses the selection when a window tab's reveal gate lifts with the caret on <body>", () => {
            setTabSwitching(true);
            runFrames(1);
            expect(giveFocus).not.toHaveBeenCalled();
            setTabSwitching(false);
            runFrames(1);
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });

        it("focuses the selection when a pane's reveal gate lifts", () => {
            setGatingNodeIds(new Set(["node-1"]));
            runFrames(1);
            expect(giveFocus).not.toHaveBeenCalled();
            setGatingNodeIds(new Set<string>());
            runFrames(1);
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });

        it("leaves a caret the user placed alone when a gate lifts", () => {
            document.body.innerHTML = `<input id="x" />`;
            document.body.querySelector<HTMLInputElement>("#x")!.focus();
            setTabSwitching(true);
            setTabSwitching(false);
            runFrames(1);
            expect(giveFocus).not.toHaveBeenCalled();
        });

        it("focuses the newly active window tab's selected pane", () => {
            setActiveTabId("tab-2");
            expect(giveFocus).not.toHaveBeenCalled();
            runFrames(1);
            expect(giveFocus).toHaveBeenCalledTimes(1);
        });
    });
});
