// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** PeekOverlay's state: its signals, and the plain variables the hover bridge, the placement and the tracking share. */

import { useHiddenTabMarker } from "@/app/platform/pane-overlay";
import { createMemo, createSignal, type JSX } from "solid-js";

export interface PeekOverlayProps {
    /** Whether the overlay should be mounted right now. */
    show: boolean;
    /**
     * Getter for the hovered row this overlay is anchored to. A getter, not
     * a plain value — the row's own `ref` callback assigns the caller's
     * local `let rowEl` variable AFTER this component's props are first
     * evaluated, so a plain value would freeze at `undefined` forever.
     * `() => rowEl` re-reads the caller's closure on every call.
     */
    rowEl: () => HTMLElement | undefined;
    /**
     * Extra class appended alongside the base `.agent-node-peek-overlay`
     * chrome — for callers whose content needs its own distinct visual
     * identity without forking the whole component.
     */
    class?: string;
    children?: JSX.Element;
}

export function createPeekState(props: PeekOverlayProps) {
    // Portal-mounted: drop its pane hole while its window tab is hidden (pane-overlay.ts).
    const hiddenTab = useHiddenTabMarker();
    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
    });

    // True when the panel reaches past the pane (wider than the row, or taller
    // than the transcript). Only then does it carry `data-pane-overlay`, so the
    // browser-pane airspace cut (platform/pane-overlay-auto.ts) is not driven for
    // every short peek.
    const [outside, setOutside] = createSignal(false);

    // `open` is the ONLY thing `<Show>` reads, written synchronously by a
    // computation (see usePeekLinger). Deriving "show || lingering" from two signals instead made the
    // panel unmount for one update (show false, linger not yet set) and remount
    // when the linger arrived — a flicker of its own.
    // `show` read through one memo owned by this component: a caller's
    // `show={a() && b()}` compiles to a getter that makes a new memo on every
    // read, and a read from a timer or an event handler (no owner) leaks it.
    const show = createMemo(() => props.show);
    const [open, setOpen] = createSignal(show());

    return {
        props,
        hiddenTab,
        floatingStyle,
        setFloatingStyle,
        outside,
        setOutside,
        show,
        open,
        setOpen,
        /** True when the last `update()` had to cut the panel's height, so it scrolls.
         *  Only such a panel is meant to be ENTERED (scroll bar, text selection). */
        lastEnterable: false,
        /** The pointer is on the panel. */
        held: false,
        bridgeTimer: undefined as ReturnType<typeof setTimeout> | undefined,
        bridgeStartedAt: 0,
        closestApproach: Infinity,
        /** The row this panel bridged, remembered rather than re-read at release time,
         *  so a row element that changes meanwhile cannot leave the bridge set (which
         *  would hold off every other peek). */
        bridgedRow: undefined as HTMLElement | undefined,
        /** The row whose open-after-leave marker this panel set. */
        markedRow: undefined as HTMLElement | undefined,
        floatingEl: undefined as HTMLElement | undefined,
        cleanupAutoUpdate: null as (() => void) | null,
        /** Latest mouse Y within the hovered row, tracked continuously (not gated on `show`)
         *  so the panel already has a real position for its very first render instead of
         *  flashing at rect.top first. */
        lastMouseY: null as number | null,
        mouseMoveRaf: null as number | null,
        /** Pointer Y relative to the row's top, frozen the first time the panel turns
         *  out to be enterable (it scrolls). From then until it closes, placement uses
         *  this instead of the live pointer, so the panel holds still while the pointer
         *  moves toward it, and it still moves with the row if the transcript scrolls.
         *  Following the live pointer made a tall panel unreachable: each move toward
         *  it moved it away and cut its height again. */
        pinnedOffsetY: null as number | null,
    };
}

export type PeekState = ReturnType<typeof createPeekState>;
