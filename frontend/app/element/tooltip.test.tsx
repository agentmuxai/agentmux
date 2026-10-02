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

/**
 * `immediate`: no delay and no fade. The panel node mounts synchronously on hover
 * whatever the delay, so these assert on its own opacity and transition (reached
 * through `data-pane-overlay`), which is what actually distinguishes the modes.
 */
describe("Tooltip immediate", () => {
    const frame = () => new Promise((resolve) => setTimeout(resolve, 60));
    const overlay = () => document.body.querySelector("[data-pane-overlay]") as HTMLElement | null;

    afterEach(() => {
        cleanup();
        vi.mocked(computePosition).mockReset();
        vi.mocked(computePosition).mockImplementation(() => Promise.resolve({ x: 0, y: 0 }) as never);
    });

    it("is fully visible one frame after hover, with no fade, where the default is still hidden", async () => {
        render(() => (
            <Tooltip immediate content={<span>now</span>}>
                <span>anchor-now</span>
            </Tooltip>
        ));
        render(() => (
            <Tooltip content={<span>later</span>}>
                <span>anchor-later</span>
            </Tooltip>
        ));
        fireEvent.mouseEnter(screen.getByText("anchor-now").parentElement!);
        fireEvent.mouseEnter(screen.getByText("anchor-later").parentElement!);
        await frame();
        const now = screen.getByText("now").closest("[data-pane-overlay]") as HTMLElement;
        const later = screen.getByText("later").closest("[data-pane-overlay]") as HTMLElement;
        expect(now.getAttribute("style")).toContain("opacity: 1");
        expect(now.getAttribute("style")).toContain("transition: none");
        // 60 ms is far under the default's 300 ms show delay.
        expect(later.getAttribute("style")).toContain("opacity: 0");
        expect(later.getAttribute("style")).toContain("opacity 200ms");
    });

    it("is gone the moment the pointer leaves, where the default lingers for its delay", async () => {
        render(() => (
            <Tooltip immediate content={<span>now</span>}>
                <span>anchor-now</span>
            </Tooltip>
        ));
        render(() => (
            <Tooltip content={<span>later</span>}>
                <span>anchor-later</span>
            </Tooltip>
        ));
        const a = screen.getByText("anchor-now").parentElement!;
        const b = screen.getByText("anchor-later").parentElement!;
        fireEvent.mouseEnter(a);
        fireEvent.mouseEnter(b);
        await frame();
        fireEvent.mouseLeave(a);
        fireEvent.mouseLeave(b);
        await tick();
        expect(screen.queryByText("now")).toBeNull();
        expect(screen.queryByText("later")).toBeInTheDocument();
    });

    // Without this the panel would show at its unpositioned top-left corner for a
    // frame, because it mounts at left:0/top:0 before the position is computed.
    it("never shows before it has been positioned, and shows as soon as it is", async () => {
        let place: () => void = () => {};
        vi.mocked(computePosition).mockImplementation(
            () =>
                new Promise((resolve) => {
                    place = () => resolve({ x: 5, y: 6 } as never);
                }),
        );
        render(() => (
            <Tooltip immediate content={<span>now</span>}>
                <span>anchor-now</span>
            </Tooltip>
        ));
        fireEvent.mouseEnter(screen.getByText("anchor-now").parentElement!);
        await frame();
        expect(overlay()!.getAttribute("style")).toContain("opacity: 0");
        place();
        await tick();
        expect(overlay()!.getAttribute("style")).toContain("opacity: 1");
        expect(overlay()!.getAttribute("style")).toContain("left: 5px");
    });
});
