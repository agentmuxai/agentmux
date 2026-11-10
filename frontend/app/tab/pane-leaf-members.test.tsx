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
import { renderPaneMembers } from "./pane-leaf-members";

vi.mock("@/app/block/block", () => ({
    Block: () => <div />,
}));
vi.mock("@/app/store/block-component-registry", () => ({
    setKeepAliveBlockDormant: () => {},
}));

const slotOf = (container: HTMLElement, i: number) =>
    container.querySelectorAll<HTMLElement>(".pane-leaf-keepalive-slot")[i];

function mountSlots(ids: string[], initialActive: string) {
    const [active, setActive] = createSignal(initialActive);
    const view = render(() =>
        renderPaneMembers({
            keepAlive: () => true,
            scopedNodeModel: () => ({}) as NodeModel,
            mountedBlockIds: () => ids,
            activeBlockId: active,
            keepAliveNodeModelFor: (id) => ({ blockId: id }) as unknown as NodeModel,
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
