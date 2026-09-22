// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tooltip — hover-triggered Portal-rendered panel.
 *
 * Covers the pointer-drag-state gate added alongside
 * `useNodePeek`'s PeekOverlay gate: a text-selection drag sweeping the
 * cursor across a tooltip-wrapped anchor must not mount/unmount this
 * Portal under the cursor mid-drag. See
 * docs/plans/PLAN_AGENT_PANE_TEXT_SELECTION_DRAG_FLICKER_2026_09_20.md.
 *
 * `isOpen` (which mounts the `<Show>`/Portal) flips synchronously inside
 * the effect driven by `isHovering()` — it does not itself wait on
 * `delayMs` (only the opacity fade-in does) — but Solid's effects flush
 * via a microtask, so each assertion needs a tick after `fireEvent` for
 * that effect to have run.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { autoUpdate, computePosition } from "@floating-ui/dom";
import { Tooltip } from "./tooltip";
import * as pointerDragState from "@/app/util/pointer-drag-state";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
    computePosition: vi.fn(() => Promise.resolve({ x: 0, y: 0 })),
    flip: vi.fn(() => ({})),
    offset: vi.fn(() => ({})),
    shift: vi.fn(() => ({})),
}));

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("Tooltip", () => {
    afterEach(() => {
        cleanup();
        vi.restoreAllMocks();
        vi.mocked(autoUpdate).mockClear();
        vi.mocked(computePosition).mockClear();
    });

    it("opens on hover", async () => {
        render(() => (
            <Tooltip content={<span>tip content</span>}>
                <span>anchor</span>
            </Tooltip>
        ));
        fireEvent.mouseEnter(screen.getByText("anchor").parentElement!);
        await tick();
        expect(screen.getByText("tip content")).toBeInTheDocument();
    });

    it("does not open on hover while the primary button is held", async () => {
        vi.spyOn(pointerDragState, "isPrimaryButtonDown").mockReturnValue(true);
        render(() => (
            <Tooltip content={<span>tip content</span>}>
                <span>anchor</span>
            </Tooltip>
        ));
        fireEvent.mouseEnter(screen.getByText("anchor").parentElement!);
        await tick();
        expect(screen.queryByText("tip content")).toBeNull();
    });

    it("still closes an already-open tooltip on leave even while the primary button is held (reagentx P1 on PR #3470 — a frozen leave got stuck open forever)", async () => {
        // delayMs=0 so the close path's own hideTimeout (unrelated to this
        // fix — it's the existing fade-out grace period) resolves in the
        // same tick as the gate check being asserted here.
        const spy = vi.spyOn(pointerDragState, "isPrimaryButtonDown").mockReturnValue(false);
        render(() => (
            <Tooltip content={<span>tip content</span>} delayMs={0}>
                <span>anchor</span>
            </Tooltip>
        ));
        fireEvent.mouseEnter(screen.getByText("anchor").parentElement!);
        await tick();
        expect(screen.getByText("tip content")).toBeInTheDocument();

        spy.mockReturnValue(true);
        fireEvent.mouseLeave(screen.getByText("anchor").parentElement!);
        await tick();
        expect(screen.queryByText("tip content")).toBeNull();
    });

    it("resumes normal hover behavior once the button is released", async () => {
        const spy = vi.spyOn(pointerDragState, "isPrimaryButtonDown").mockReturnValue(true);
        render(() => (
            <Tooltip content={<span>tip content</span>}>
                <span>anchor</span>
            </Tooltip>
        ));
        fireEvent.mouseEnter(screen.getByText("anchor").parentElement!);
        await tick();
        expect(screen.queryByText("tip content")).toBeNull();

        spy.mockReturnValue(false);
        fireEvent.mouseEnter(screen.getByText("anchor").parentElement!);
        await tick();
        expect(screen.getByText("tip content")).toBeInTheDocument();
    });
});
