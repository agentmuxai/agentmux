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
import { createSignal } from "solid-js";
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

/** A tooltip is hovered inside dialogs, popovers and menus, so it takes the tooltip layer, not a number of its own. */
describe("Tooltip layer", () => {
    afterEach(cleanup);

    it("stacks on the tooltip layer, over dialogs and menus", async () => {
        render(() => (
            <Tooltip content={<span>tip content</span>}>
                <span>anchor</span>
            </Tooltip>
        ));
        fireEvent.mouseEnter(screen.getByText("anchor").parentElement!);
        await tick();
        const panel = screen.getByText("tip content").parentElement!;
        expect(panel.className).toContain("z-[var(--z-tooltip)]");
        expect(panel.className).not.toMatch(/(^|\s)z-\d+(\s|$)/);
    });
});

/** The default: near-instant, warm between neighbours, a short grace on leave. */
describe("Tooltip default timing", () => {
    const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
    const overlays = () => [...document.body.querySelectorAll<HTMLElement>("[data-pane-overlay]")];

    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    function two() {
        render(() => (
            <>
                <Tooltip content={<span>first</span>}>
                    <span>anchor-first</span>
                </Tooltip>
                <Tooltip content={<span>second</span>}>
                    <span>anchor-second</span>
                </Tooltip>
            </>
        ));
        return [screen.getByText("anchor-first").parentElement!, screen.getByText("anchor-second").parentElement!];
    }

    it("shows well under the old 300 ms, not at the very instant of entering", async () => {
        const [a] = two();
        // Let the previous test's close count as long ago.
        await wait(450);
        fireEvent.mouseEnter(a);
        await tick();
        expect(overlays()[0].getAttribute("style")).toContain("opacity: 0");
        await wait(100);
        expect(overlays()[0].getAttribute("style")).toContain("opacity: 1");
    });

    it("lingers briefly after the pointer leaves, then goes", async () => {
        const [a] = two();
        await wait(450);
        fireEvent.mouseEnter(a);
        await wait(100);
        fireEvent.mouseLeave(a);
        await wait(30);
        expect(screen.queryByText("first")).toBeInTheDocument();
        await wait(150);
        expect(screen.queryByText("first")).toBeNull();
    });

    it("shows the next tooltip at once when moving from one anchor to the next, and closes the first", async () => {
        vi.useFakeTimers();
        const [a, b] = two();
        fireEvent.mouseEnter(a);
        await vi.advanceTimersByTimeAsync(500);
        fireEvent.mouseLeave(a);
        fireEvent.mouseEnter(b);
        // One frame to place it: less than the 40 ms rest a cold tooltip waits.
        await vi.advanceTimersByTimeAsync(20);
        expect(screen.queryByText("first")).toBeNull();
        const second = screen.getByText("second").closest("[data-pane-overlay]") as HTMLElement;
        expect(second.getAttribute("style")).toContain("opacity: 1");
    });

    it("does not stay 'warm' for the others after an open tooltip is disabled", async () => {
        vi.useFakeTimers();
        const [disabled, setDisabled] = createSignal(false);
        render(() => (
            <>
                <Tooltip content={<span>first</span>} disable={disabled()}>
                    <span>anchor-first</span>
                </Tooltip>
                <Tooltip content={<span>second</span>}>
                    <span>anchor-second</span>
                </Tooltip>
            </>
        ));
        fireEvent.mouseEnter(screen.getByText("anchor-first").parentElement!);
        await vi.advanceTimersByTimeAsync(100);
        setDisabled(true);
        await vi.advanceTimersByTimeAsync(1000);
        fireEvent.mouseLeave(screen.getByText("anchor-first").parentElement!);
        fireEvent.mouseEnter(screen.getByText("anchor-second").parentElement!);
        await vi.advanceTimersByTimeAsync(20);
        expect(overlays()[0].getAttribute("style")).toContain("opacity: 0");
    });

    it("is invisible again until it has been placed when it reopens", async () => {
        vi.useFakeTimers();
        const [a] = two();
        fireEvent.mouseEnter(a);
        await vi.advanceTimersByTimeAsync(100);
        expect(overlays()[0].getAttribute("style")).toContain("opacity: 1");
        fireEvent.mouseLeave(a);
        await vi.advanceTimersByTimeAsync(200);
        expect(overlays()).toHaveLength(0);
        // Reopen with the placement not yet computed.
        vi.mocked(computePosition).mockImplementationOnce(() => new Promise(() => {}));
        fireEvent.mouseEnter(a);
        await vi.advanceTimersByTimeAsync(100);
        expect(overlays()[0].getAttribute("style")).toContain("opacity: 0");
    });

    it("waits the short rest when no tooltip has been open lately", async () => {
        vi.useFakeTimers();
        const [a] = two();
        await vi.advanceTimersByTimeAsync(1000);
        fireEvent.mouseEnter(a);
        await vi.advanceTimersByTimeAsync(20);
        expect(overlays()[0].getAttribute("style")).toContain("opacity: 0");
        await vi.advanceTimersByTimeAsync(40);
        expect(overlays()[0].getAttribute("style")).toContain("opacity: 1");
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

    it("is fully visible one frame after hover, with no fade, where the default has its short fade", async () => {
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
        // The default shows after its 40 ms rest, with the short fade token.
        expect(later.getAttribute("style")).toContain("opacity: 1");
        expect(later.getAttribute("style")).toContain("opacity var(--duration-instant)");
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
