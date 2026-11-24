// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A dormant keep-alive member skips layout while the window is resized and until its turn to catch up
 * (window-resize.ts); the active member never does. The Block is a stand-in; the slots and the window
 * resize tracking are real.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NodeModel } from "@/layout/index";
import {
    noteWindowResize,
    resetWindowResizeForTests,
    WINDOW_RESIZE_SETTLE_MS,
} from "@/app/platform/window-resize";
import { resetTabLandingForTests, showTabLanding, tabMemberLands } from "@/layout/lib/tabLanding";
import { renderPaneMembers } from "./pane-leaf-members";

vi.mock("@/app/block/block", () => ({
    Block: () => <div />,
}));
vi.mock("@/app/store/block-component-registry", () => ({
    setKeepAliveBlockDormant: () => {},
}));

const slotOf = (container: HTMLElement, i: number) =>
    container.querySelectorAll<HTMLElement>(".pane-leaf-keepalive-slot")[i];

function mountSlots(ids: string[], initialActive: string, splitterDragging: () => boolean = () => false) {
    const [active, setActive] = createSignal(initialActive);
    const view = render(() =>
        renderPaneMembers({
            keepAlive: () => true,
            scopedNodeModel: () => ({}) as NodeModel,
            mountedBlockIds: () => ids,
            activeBlockId: active,
            keepAliveNodeModelFor: (id) => ({ blockId: id, isSplitterDragging: splitterDragging }) as unknown as NodeModel,
        })
    );
    return { ...view, setActive };
}

describe("a dormant keep-alive slot during a window resize", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        resetWindowResizeForTests();
    });
    afterEach(() => {
        cleanup();
        resetWindowResizeForTests();
        vi.useRealTimers();
    });

    it("lays out normally when no resize is going on", () => {
        const { container } = mountSlots(["a", "b"], "a");
        expect(slotOf(container, 0).style.contentVisibility).toBe("");
        expect(slotOf(container, 1).style.contentVisibility).toBe("");
    });

    it("skips layout while the window resizes, never for the active member", () => {
        const { container } = mountSlots(["a", "b", "c"], "a");
        noteWindowResize();
        expect(slotOf(container, 0).style.contentVisibility).toBe("");
        expect(slotOf(container, 1).style.contentVisibility).toBe("hidden");
        expect(slotOf(container, 2).style.contentVisibility).toBe("hidden");
        // still mounted and still the same box: the slot keeps its place and size
        expect(slotOf(container, 1).style.visibility).toBe("hidden");
        expect(slotOf(container, 1).style.inset).toBe("0px");
    });

    it("catches up one slot per idle period once the window settles", () => {
        const { container } = mountSlots(["a", "b", "c"], "a");
        noteWindowResize();
        vi.advanceTimersByTime(WINDOW_RESIZE_SETTLE_MS);
        // settled, but neither dormant slot has had its turn yet
        expect(slotOf(container, 1).style.contentVisibility).toBe("hidden");
        expect(slotOf(container, 2).style.contentVisibility).toBe("hidden");
        vi.advanceTimersByTime(16);
        expect(slotOf(container, 1).style.contentVisibility).toBe("");
        expect(slotOf(container, 2).style.contentVisibility).toBe("hidden");
        vi.advanceTimersByTime(16);
        expect(slotOf(container, 2).style.contentVisibility).toBe("");
    });

    it("lays a member out at once when it becomes the active one mid-resize", () => {
        const { container, setActive } = mountSlots(["a", "b"], "a");
        noteWindowResize();
        expect(slotOf(container, 1).style.contentVisibility).toBe("hidden");
        setActive("b");
        expect(slotOf(container, 1).style.contentVisibility).toBe("");
        expect(slotOf(container, 0).style.contentVisibility).toBe("hidden");
    });
});

describe("a dormant keep-alive slot during a splitter drag", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        resetWindowResizeForTests();
    });
    afterEach(() => {
        cleanup();
        resetWindowResizeForTests();
        vi.useRealTimers();
    });

    it("skips layout while a splitter is dragged, never for the active member, however many tabs there are", () => {
        const [dragging, setDragging] = createSignal(false);
        const ids = Array.from({ length: 30 }, (_, i) => `m${i}`);
        const { container } = mountSlots(ids, "m0", dragging);
        setDragging(true);
        const slots = Array.from(container.querySelectorAll<HTMLElement>(".pane-leaf-keepalive-slot"));
        expect(slots[0].style.contentVisibility).toBe("");
        expect(slots.slice(1).every((s) => s.style.contentVisibility === "hidden")).toBe(true);
        // still mounted, with the same box
        expect(slots[1].style.visibility).toBe("hidden");
        expect(slots[1].style.inset).toBe("0px");
    });

    it("catches up one slot per idle period once the drag ends", () => {
        const [dragging, setDragging] = createSignal(false);
        const { container } = mountSlots(["a", "b", "c"], "a", dragging);
        setDragging(true);
        setDragging(false);
        expect(slotOf(container, 1).style.contentVisibility).toBe("hidden");
        expect(slotOf(container, 2).style.contentVisibility).toBe("hidden");
        vi.advanceTimersByTime(16);
        expect(slotOf(container, 1).style.contentVisibility).toBe("");
        expect(slotOf(container, 2).style.contentVisibility).toBe("hidden");
        vi.advanceTimersByTime(16);
        expect(slotOf(container, 2).style.contentVisibility).toBe("");
    });

    it("does not catch up while the window is resizing, but when that settles", () => {
        const [dragging, setDragging] = createSignal(false);
        const { container } = mountSlots(["a", "b"], "a", dragging);
        setDragging(true);
        noteWindowResize();
        setDragging(false);
        vi.advanceTimersByTime(16);
        expect(slotOf(container, 1).style.contentVisibility).toBe("hidden");
        vi.advanceTimersByTime(WINDOW_RESIZE_SETTLE_MS + 16);
        expect(slotOf(container, 1).style.contentVisibility).toBe("");
    });

    it("lays a member out at once when it becomes the active one mid-drag", () => {
        const [dragging, setDragging] = createSignal(false);
        const { container, setActive } = mountSlots(["a", "b"], "a", dragging);
        setDragging(true);
        expect(slotOf(container, 1).style.contentVisibility).toBe("hidden");
        setActive("b");
        expect(slotOf(container, 1).style.contentVisibility).toBe("");
        expect(slotOf(container, 0).style.contentVisibility).toBe("hidden");
    });
});
describe("a member whose tab is dragged out of the pane", () => {
    afterEach(() => {
        cleanup();
        resetTabLandingForTests();
    });

    const memberOf = (slot: HTMLElement) => slot.querySelector<HTMLElement>(".pane-leaf-member");

    it("stands in its landing place while the place is shown, and is back in its slot when the place goes", () => {
        const { container } = mountSlots(["a", "b"], "b");
        const member = memberOf(slotOf(container, 0))!;
        expect(tabMemberLands("a")).toBe(true);
        const place = document.createElement("div");
        const hide = showTabLanding("a", place);
        // The same element, moved: nothing is mounted twice.
        expect(member.parentElement).toBe(place);
        expect(memberOf(slotOf(container, 0))).toBeNull();
        // The other member stays where it is, and the slot itself never moves.
        expect(memberOf(slotOf(container, 1))).not.toBeNull();
        expect(slotOf(container, 0).parentElement).toBe(slotOf(container, 1).parentElement);
        hide();
        expect(member.parentElement).toBe(slotOf(container, 0));
    });

    it("leaves the place with its slot when the tab leaves the pane while the place is shown", () => {
        const [ids, setIds] = createSignal(["a", "b"]);
        const { container } = render(() =>
            renderPaneMembers({
                keepAlive: () => true,
                scopedNodeModel: () => ({}) as NodeModel,
                mountedBlockIds: ids,
                activeBlockId: () => "b",
                keepAliveNodeModelFor: (id) => ({ blockId: id }) as unknown as NodeModel,
            })
        );
        const place = document.createElement("div");
        showTabLanding("a", place);
        expect(place.childElementCount).toBe(1);
        setIds(["b"]);
        expect(place.childElementCount).toBe(0);
        expect(container.querySelectorAll(".pane-leaf-keepalive-slot").length).toBe(1);
        expect(tabMemberLands("a")).toBe(false);
    });

    it("does not move a member that cannot stand there (a native surface)", () => {
        const { container } = render(() =>
            renderPaneMembers({
                keepAlive: () => true,
                scopedNodeModel: () => ({}) as NodeModel,
                mountedBlockIds: () => ["a"],
                activeBlockId: () => "a",
                keepAliveNodeModelFor: (id) => ({ blockId: id }) as unknown as NodeModel,
                lands: () => false,
            })
        );
        expect(tabMemberLands("a")).toBe(false);
        const place = document.createElement("div");
        showTabLanding("a", place);
        expect(place.childElementCount).toBe(0);
        expect(memberOf(slotOf(container, 0))).not.toBeNull();
    });
});
