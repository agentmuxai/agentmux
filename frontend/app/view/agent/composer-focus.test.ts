// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * composer-focus — SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §3.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    cancelComposerFocusRequest,
    focusComposer,
    focusComposerWhenReady,
    LAUNCH_FOCUS_WINDOW_MS,
    requestComposerFocus,
    takeComposerFocusRequest,
} from "./composer-focus";

// A block shaped like the real one: [data-blockid] root holding the hidden
// dummy-focus input, a transcript, a picker button, and the composer.
function makeBlock(blockId: string) {
    const block = document.createElement("div");
    block.setAttribute("data-blockid", blockId);
    const content = document.createElement("div");
    const dummy = document.createElement("input");
    dummy.id = `${blockId}-dummy-focus`;
    const transcript = document.createElement("p");
    transcript.textContent = "some transcript text to select";
    const pickerButton = document.createElement("button");
    pickerButton.textContent = "Maks";
    const ta = document.createElement("textarea");
    content.append(transcript, pickerButton, ta);
    block.append(dummy, content);
    document.body.append(block);
    return { block, content, dummy, transcript, pickerButton, ta };
}

describe("composer focus requests", () => {
    it("a request is consumed exactly once", () => {
        requestComposerFocus("b1");
        expect(takeComposerFocusRequest("b1")).toBe(true);
        expect(takeComposerFocusRequest("b1")).toBe(false);
    });

    it("a cancelled request is not honored", () => {
        requestComposerFocus("b2");
        cancelComposerFocusRequest("b2");
        expect(takeComposerFocusRequest("b2")).toBe(false);
    });

    it("a stale request (never consumed by a mount) expires", () => {
        vi.useFakeTimers();
        try {
            requestComposerFocus("b3");
            vi.advanceTimersByTime(11_000);
            expect(takeComposerFocusRequest("b3")).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });

    it("requests are per block", () => {
        requestComposerFocus("b4");
        expect(takeComposerFocusRequest("other")).toBe(false);
        expect(takeComposerFocusRequest("b4")).toBe(true);
    });
});

describe("focusComposer (giveFocus)", () => {
    afterEach(() => {
        document.getSelection()?.removeAllRanges();
        document.body.innerHTML = "";
    });

    it("focuses the textarea with the caret at the end", () => {
        const { ta } = makeBlock("blk");
        ta.value = "draft";
        expect(focusComposer(ta)).toBe(true);
        expect(document.activeElement).toBe(ta);
        expect(ta.selectionStart).toBe(5);
        expect(ta.selectionEnd).toBe(5);
    });

    it("never lets the browser scroll ancestors to reveal the composer (preventScroll)", () => {
        // REPORT_TAB_PANES_OFFSET_HALF_WINDOW_2026_09_24.md: a plain focus()
        // scrolled .tile-layout by half its height, shifting a whole tab.
        const { ta } = makeBlock("blk");
        const focus = vi.spyOn(ta, "focus");
        focusComposer(ta);
        expect(focus).toHaveBeenCalledTimes(1);
        expect(focus).toHaveBeenCalledWith({ preventScroll: true });
    });

    it("steals nothing from the dummy input — it is a fallback, not a real owner", () => {
        const { ta, dummy } = makeBlock("blk");
        dummy.focus();
        expect(focusComposer(ta)).toBe(true);
        expect(document.activeElement).toBe(ta);
    });

    it("returns false (caller falls back) when the composer is under an inert ancestor", () => {
        const { ta, content } = makeBlock("blk");
        content.setAttribute("inert", "");
        expect(focusComposer(ta)).toBe(false);
        expect(document.activeElement).not.toBe(ta);
    });

    it("returns false and keeps a non-collapsed text selection inside the block", () => {
        const { ta, transcript } = makeBlock("blk");
        const range = document.createRange();
        range.selectNodeContents(transcript);
        document.getSelection()!.addRange(range);
        expect(focusComposer(ta)).toBe(false);
        expect(document.activeElement).not.toBe(ta);
        expect(document.getSelection()!.isCollapsed).toBe(false);
    });

    it("leaves another text entry in the same block focused (login/decision panel) and reports handled", () => {
        const { ta, content } = makeBlock("blk");
        const panelInput = document.createElement("input");
        content.append(panelInput);
        panelInput.focus();
        expect(focusComposer(ta)).toBe(true);
        expect(document.activeElement).toBe(panelInput);
    });
});

describe("focusComposerWhenReady (launch path)", () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });
    afterEach(() => {
        vi.useRealTimers();
        document.body.innerHTML = "";
    });

    it("picker click: moves focus from the clicked My Agents button straight into the composer", () => {
        const { ta, pickerButton } = makeBlock("blk");
        pickerButton.focus();
        focusComposerWhenReady(ta, "blk");
        expect(document.activeElement).toBe(ta);
        // The picker unmounting afterwards doesn't matter — focus already moved.
        pickerButton.remove();
        expect(document.activeElement).toBe(ta);
    });

    it("focus already dropped to <body>: still lands in the composer", () => {
        const { ta } = makeBlock("blk");
        (document.activeElement as HTMLElement | null)?.blur();
        focusComposerWhenReady(ta, "blk");
        expect(document.activeElement).toBe(ta);
    });

    it("launch modal: waits while the content is inert, then focuses once the modal closes", () => {
        const { block, ta, content } = makeBlock("blk");
        content.setAttribute("inert", "");
        const modal = document.createElement("div");
        modal.className = "modal-root";
        const launchButton = document.createElement("button");
        modal.append(launchButton);
        block.append(modal);
        launchButton.focus();

        focusComposerWhenReady(ta, "blk");
        vi.advanceTimersByTime(500);
        expect(document.activeElement).toBe(launchButton);

        // Modal closes: inert released, modal removed.
        content.removeAttribute("inert");
        modal.remove();
        vi.advanceTimersByTime(100);
        expect(document.activeElement).toBe(ta);
    });

    it("gives up if the user moved to another pane during the launch", () => {
        const { ta } = makeBlock("blk");
        const other = makeBlock("other-blk");
        other.ta.focus();
        focusComposerWhenReady(ta, "blk");
        vi.advanceTimersByTime(1000);
        expect(document.activeElement).toBe(other.ta);
    });

    it("gives up if a text entry outside any pane has focus (the user is typing elsewhere)", () => {
        const { ta } = makeBlock("blk");
        const outside = document.createElement("input");
        document.body.append(outside);
        outside.focus();
        focusComposerWhenReady(ta, "blk");
        vi.advanceTimersByTime(1000);
        expect(document.activeElement).toBe(outside);
    });

    it("stops trying after the launch window", () => {
        const { ta, content } = makeBlock("blk");
        content.setAttribute("inert", "");
        focusComposerWhenReady(ta, "blk");
        vi.advanceTimersByTime(LAUNCH_FOCUS_WINDOW_MS + 500);
        content.removeAttribute("inert");
        vi.advanceTimersByTime(1000);
        expect(document.activeElement).not.toBe(ta);
    });

    it("cancel (footer unmount) stops a pending attempt", () => {
        const { ta, content } = makeBlock("blk");
        content.setAttribute("inert", "");
        const cancel = focusComposerWhenReady(ta, "blk");
        cancel();
        content.removeAttribute("inert");
        vi.advanceTimersByTime(1000);
        expect(document.activeElement).not.toBe(ta);
    });
});
