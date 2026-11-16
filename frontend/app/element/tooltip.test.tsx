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

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { autoUpdate, computePosition } from "@floating-ui/dom";
import { createSignal, Show } from "solid-js";
import { Tooltip } from "./tooltip";
import * as pointerDragState from "@/app/util/pointer-drag-state";

vi.mock("@floating-ui/dom", () => ({
    // The real autoUpdate places the panel at once; AnchoredPopover places it only through it.
    autoUpdate: vi.fn((_reference: unknown, _floating: unknown, update: () => void) => {
        update();
        return vi.fn();
    }),
    computePosition: vi.fn(() => Promise.resolve({ x: 0, y: 0 })),
    flip: vi.fn(() => ({})),
    offset: vi.fn(() => ({})),
    shift: vi.fn(() => ({})),
    size: vi.fn(() => ({})),
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

/** A press is the person acting on the anchor, not reading it, as with the tooltip for titled elements. */
describe("Tooltip and a press on its anchor", () => {
    afterEach(() => {
        cleanup();
        // Lets go of the button and of the press point, which the pointer state keeps for the whole page.
        fireEvent.pointerUp(window);
        window.dispatchEvent(new Event("blur"));
    });

    const AT = { clientX: 100, clientY: 20 };
    const AWAY = { clientX: 300, clientY: 200 };
    const mount = (inner = <button type="button">anchor</button>, forceOpen?: () => boolean) => {
        render(() => (
            <Tooltip content={<span>tip content</span>} forceOpen={forceOpen?.()}>
                {inner}
            </Tooltip>
        ));
        return screen.getByText("anchor").parentElement!;
    };
    const click = (el: Element) => {
        fireEvent.pointerDown(el, AT);
        fireEvent.pointerUp(el, AT);
        fireEvent.click(el, AT);
    };

    it("goes at once when the anchor is pressed, and stays shut while the pointer rests there", async () => {
        const anchor = mount();
        fireEvent.mouseEnter(anchor);
        await tick();
        expect(screen.getByText("tip content")).toBeInTheDocument();

        fireEvent.pointerDown(screen.getByText("anchor"), AT);
        await tick();
        // No linger: the panel is out of the page, not fading over what the press opened.
        expect(screen.queryByText("tip content")).toBeNull();

        fireEvent.pointerUp(screen.getByText("anchor"), AT);
        fireEvent.click(screen.getByText("anchor"), AT);
        await new Promise((resolve) => setTimeout(resolve, 150));
        expect(screen.queryByText("tip content")).toBeNull();
    });

    it("shows again once the pointer has left the anchor and come back", async () => {
        const anchor = mount();
        fireEvent.mouseEnter(anchor, AT);
        click(anchor);
        await tick();
        expect(screen.queryByText("tip content")).toBeNull();

        fireEvent.mouseLeave(anchor, AWAY);
        fireEvent.pointerMove(document.body, AWAY);
        fireEvent.mouseEnter(anchor, AT);
        await tick();
        expect(screen.getByText("tip content")).toBeInTheDocument();
    });

    // The top bar redraws its widgets when one is clicked: the new element is under a pointer that has not
    // moved, and the browser reports it as entered.
    it("stays shut on an anchor that the click put under the resting pointer", async () => {
        const [round, setRound] = createSignal(1);
        render(() => (
            <Show when={round()} keyed>
                {(n) => (
                    <Tooltip content={<span>tip content</span>}>
                        <button type="button" onClick={() => setRound(n + 1)}>
                            anchor {n}
                        </button>
                    </Tooltip>
                )}
            </Show>
        ));
        const first = screen.getByText("anchor 1");
        fireEvent.mouseEnter(first.parentElement!, AT);
        await tick();
        expect(screen.getByText("tip content")).toBeInTheDocument();

        click(first);
        const second = screen.getByText("anchor 2");
        expect(second).not.toBe(first);
        // A few pixels of drift during the click are still resting.
        fireEvent.mouseEnter(second.parentElement!, { clientX: AT.clientX + 2, clientY: AT.clientY - 1 });
        await new Promise((resolve) => setTimeout(resolve, 150));
        expect(screen.queryByText("tip content")).toBeNull();

        fireEvent.mouseLeave(second.parentElement!, AWAY);
        fireEvent.pointerMove(document.body, AWAY);
        fireEvent.mouseEnter(second.parentElement!, AT);
        await tick();
        expect(screen.getByText("tip content")).toBeInTheDocument();
    });

    it("shows on another anchor reached in one fast move from the press, whose enter comes before its move", async () => {
        render(() => (
            <>
                <Tooltip content={<span>first tip</span>}>
                    <button type="button">one</button>
                </Tooltip>
                <Tooltip content={<span>second tip</span>}>
                    <button type="button">two</button>
                </Tooltip>
            </>
        ));
        const one = screen.getByText("one").parentElement!;
        const two = screen.getByText("two").parentElement!;
        fireEvent.mouseEnter(one, AT);
        click(one);
        fireEvent.mouseLeave(one, AWAY);
        fireEvent.mouseEnter(two, AWAY);
        await tick();
        expect(screen.getByText("second tip")).toBeInTheDocument();
    });

    it("sees the press even when the control inside keeps the event to itself", async () => {
        const anchor = mount(
            <button type="button" on:pointerdown={(e) => e.stopPropagation()}>
                anchor
            </button>,
        );
        fireEvent.mouseEnter(anchor, AT);
        await tick();
        fireEvent.pointerDown(screen.getByText("anchor"), AT);
        await tick();
        expect(screen.queryByText("tip content")).toBeNull();
    });

    it("closes for a press with any button, such as the one that opens a context menu", async () => {
        const anchor = mount();
        fireEvent.mouseEnter(anchor, AT);
        await tick();
        fireEvent.pointerDown(anchor, { ...AT, button: 2 });
        await tick();
        expect(screen.queryByText("tip content")).toBeNull();
    });

    it("stays while it is held open, as a chip's detail is after a tap", async () => {
        const [held, setHeld] = createSignal(false);
        const anchor = mount(<span>anchor</span>, held);
        fireEvent.mouseEnter(anchor, AT);
        fireEvent.pointerDown(anchor, AT);
        setHeld(true);
        await tick();
        expect(screen.getByText("tip content")).toBeInTheDocument();

        // The hold ends with the pointer still on the pressed anchor: it does not fall back to a hover tooltip.
        setHeld(false);
        await tick();
        expect(screen.queryByText("tip content")).toBeNull();
    });

    it("shows for a tap, whose hover arrives after its press and at the same point", async () => {
        const anchor = mount();
        fireEvent.pointerDown(anchor, { ...AT, pointerType: "touch" });
        fireEvent.pointerUp(anchor, { ...AT, pointerType: "touch" });
        fireEvent.mouseEnter(anchor, AT);
        await tick();
        expect(screen.getByText("tip content")).toBeInTheDocument();
    });
});

/** The panel's placement lives on the shell AnchoredPopover places, its opacity and fade on the body inside it. */
const styleOf = (shell: Element | null): string =>
    `${shell?.getAttribute("style") ?? ""};${shell?.firstElementChild?.getAttribute("style") ?? ""}`;

/** A tooltip is hovered inside dialogs, popovers and menus, so it takes the tooltip layer, not a number of its own. */
describe("Tooltip layer", () => {
    afterEach(cleanup);

    it("takes no pointer events when it is a passive status, so it can't end the hover that opened it", async () => {
        render(() => (
            <Tooltip passive content={<span>status</span>}>
                <span>anchor</span>
            </Tooltip>
        ));
        fireEvent.mouseEnter(screen.getByText("anchor").parentElement!);
        await tick();
        expect(screen.getByText("status").closest(".anchored-popover")!.className).toContain("anchored-popover--passive");
        expect(readFileSync(join(process.cwd(), "frontend/app/element/anchored-popover.scss"), "utf8")).toMatch(
            /anchored-popover--passive [{][^}]*pointer-events: none/,
        );
    });

    it("stacks on the tooltip layer, over dialogs and menus", async () => {
        render(() => (
            <Tooltip content={<span>tip content</span>}>
                <span>anchor</span>
            </Tooltip>
        ));
        fireEvent.mouseEnter(screen.getByText("anchor").parentElement!);
        await tick();
        const shell = screen.getByText("tip content").closest(".anchored-popover")!;
        expect(shell.className).toContain("anchored-popover--tooltip");
        expect(shell.className).not.toMatch(/(^|\s)z-\d+(\s|$)/);
        // The class takes the tooltip layer, the same as the Tailwind class it replaces.
        expect(readFileSync(join(process.cwd(), "frontend/app/element/anchored-popover.scss"), "utf8")).toMatch(
            /\.anchored-popover--tooltip \{\s*z-index: var\(--z-tooltip\);/,
        );
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
        expect(styleOf(overlays()[0])).toContain("opacity: 0");
        await wait(100);
        expect(styleOf(overlays()[0])).toContain("opacity: 1");
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
        expect(styleOf(second)).toContain("opacity: 1");
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
        expect(styleOf(overlays()[0])).toContain("opacity: 0");
    });

    it("does not stay 'warm' after an immediate tooltip closed the open default one", async () => {
        vi.useFakeTimers();
        render(() => (
            <>
                <Tooltip content={<span>first</span>}>
                    <span>anchor-first</span>
                </Tooltip>
                <Tooltip immediate content={<span>quick</span>}>
                    <span>anchor-quick</span>
                </Tooltip>
                <Tooltip content={<span>third</span>}>
                    <span>anchor-third</span>
                </Tooltip>
            </>
        ));
        const [first, quick, third] = ["first", "quick", "third"].map((n) => screen.getByText(`anchor-${n}`).parentElement!);
        fireEvent.mouseEnter(first);
        await vi.advanceTimersByTimeAsync(100);
        fireEvent.mouseLeave(first);
        fireEvent.mouseEnter(quick);
        await vi.advanceTimersByTimeAsync(1000);
        fireEvent.mouseLeave(quick);
        fireEvent.mouseEnter(third);
        await vi.advanceTimersByTimeAsync(20);
        expect(styleOf(screen.getByText("third").closest("[data-pane-overlay]")!)).toContain("opacity: 0");
    });

    it("closes the open default tooltip when an immediate one opens, though it does not take its place", async () => {
        vi.useFakeTimers();
        render(() => (
            <>
                <Tooltip content={<span>first</span>}>
                    <span>anchor-first</span>
                </Tooltip>
                <Tooltip immediate content={<span>quick</span>}>
                    <span>anchor-quick</span>
                </Tooltip>
            </>
        ));
        const first = screen.getByText("anchor-first").parentElement!;
        const quick = screen.getByText("anchor-quick").parentElement!;
        fireEvent.mouseEnter(first);
        await vi.advanceTimersByTimeAsync(100);
        fireEvent.mouseLeave(first);
        fireEvent.mouseEnter(quick);
        await vi.advanceTimersByTimeAsync(20);
        // Not left for the first one's 100 ms grace.
        expect(screen.queryByText("first")).toBeNull();
        expect(screen.getByText("quick")).toBeInTheDocument();
    });

    it("is invisible again until it has been placed when it reopens", async () => {
        vi.useFakeTimers();
        const [a] = two();
        fireEvent.mouseEnter(a);
        await vi.advanceTimersByTimeAsync(100);
        expect(styleOf(overlays()[0])).toContain("opacity: 1");
        fireEvent.mouseLeave(a);
        await vi.advanceTimersByTimeAsync(200);
        expect(overlays()).toHaveLength(0);
        // Reopen with the placement not yet computed.
        vi.mocked(computePosition).mockImplementationOnce(() => new Promise(() => {}));
        fireEvent.mouseEnter(a);
        await vi.advanceTimersByTimeAsync(100);
        expect(styleOf(overlays()[0])).toContain("opacity: 0");
    });

    it("waits the short rest when no tooltip has been open lately", async () => {
        vi.useFakeTimers();
        const [a] = two();
        await vi.advanceTimersByTimeAsync(1000);
        fireEvent.mouseEnter(a);
        await vi.advanceTimersByTimeAsync(20);
        expect(styleOf(overlays()[0])).toContain("opacity: 0");
        await vi.advanceTimersByTimeAsync(40);
        expect(styleOf(overlays()[0])).toContain("opacity: 1");
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
        await frame();
        const now = screen.getByText("now").closest("[data-pane-overlay]") as HTMLElement;
        expect(styleOf(now)).toContain("opacity: 1");
        expect(styleOf(now)).toContain("transition: none");
        fireEvent.mouseLeave(screen.getByText("anchor-now").parentElement!);
        fireEvent.mouseEnter(screen.getByText("anchor-later").parentElement!);
        await frame();
        const later = screen.getByText("later").closest("[data-pane-overlay]") as HTMLElement;
        // The default shows after its 40 ms rest, with the short fade token.
        expect(styleOf(later)).toContain("opacity: 1");
        expect(styleOf(later)).toContain("opacity var(--duration-instant)");
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
        expect(styleOf(overlay()!)).toContain("opacity: 0");
        place();
        await tick();
        expect(styleOf(overlay()!)).toContain("opacity: 1");
        expect(styleOf(overlay()!)).toContain("left: 5px");
    });
});
