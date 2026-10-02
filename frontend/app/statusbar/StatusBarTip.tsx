// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Replaces the status bar's pure-CSS `[data-tip]:hover::after` tooltip
 * with a real, Portal'd DOM element that participates in the airspace-clip
 * mechanism. A CSS `::after` pseudo-element is never a real DOM node —
 * it can't be tagged `data-pane-overlay`, measured by a `ResizeObserver`,
 * or registered with `usePaneOverlay()`, so it could never paint over a
 * native browser-pane HWND (the same "airspace problem" the status-bar
 * popovers had — SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md,
 * SPEC_STATUS_BAR_POPOVER_AIRSPACE_CLIP_2026_08_17.md).
 *
 * Keeps the exact call-site API unchanged: any status-bar descendant with
 * a `data-tip="…"` attribute still gets a hover/focus-visible balloon,
 * with zero JSX changes needed at any of the existing call sites. A single
 * delegated `mouseover`/`mouseout`/`focusin`/`focusout` listener on
 * `document` (mirrors the existing delegated outside-click-to-close
 * listeners already used throughout this directory) replaces the
 * per-element `:hover`/`:focus-visible` CSS selectors — this mirrors how
 * `pane-overlay-auto.ts` made the `data-pane-overlay` clip itself
 * declarative instead of requiring a hook at every call site.
 *
 * Mount exactly once (in `StatusBar.tsx`).
 */

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import "./StatusBarTip.scss";

interface TipBalloonProps {
    target: HTMLElement;
    text: string;
}

/**
 * The balloon itself, split out so AnchoredPopover's positioning and airspace
 * cut run for ITS OWN mount lifecycle (only while a tip is showing). A cursor
 * sweeping the status bar mounts and disposes a balloon per `[data-tip]` it
 * crosses, often inside one frame; AnchoredPopover cancels a frame that has
 * not run yet, so no `autoUpdate` outlives its balloon (a leak seen live
 * 2026-09-22: 4,718 throws in one hour).
 */
const TipBalloon = (props: TipBalloonProps): JSX.Element => (
    <AnchoredPopover
        anchor={props.target}
        placement="top"
        class="status-bar-tip-balloon"
        shellClass="status-bar-tip-shell"
    >
        {props.text}
    </AnchoredPopover>
);

TipBalloon.displayName = "TipBalloon";

export const StatusBarTip = (): JSX.Element => {
    const [activeEl, setActiveEl] = createSignal<HTMLElement | null>(null);

    const findTipEl = (t: EventTarget | null): HTMLElement | null => {
        if (!(t instanceof Element)) return null;
        const el = t.closest<HTMLElement>("[data-tip]");
        if (!el) return null;
        // Scope to the status bar itself and its own portaled popovers
        // (HostPopoverPanel/BackendStatusPanel render via <Portal> to
        // document.body, so their content is no longer a DOM descendant of
        // `.status-bar` — `.status-bar-popover` is the shared marker class
        // both carry). Elsewhere in the app (e.g. the editor file-tree
        // toolbar, editor-view.scss's own `[data-tip]:hover::after` copy)
        // keeps its separate, untouched CSS tooltip — this listener is
        // deliberately document-scoped for reach into the portaled
        // popovers, not because it should own every `data-tip` in the app.
        if (!el.closest(".status-bar, .status-bar-popover")) return null;
        return el;
    };

    const onMouseOver = (e: MouseEvent) => {
        const el = findTipEl(e.target);
        if (el && el !== activeEl()) setActiveEl(el);
    };
    const onMouseOut = (e: MouseEvent) => {
        const el = findTipEl(e.target);
        if (!el || el !== activeEl()) return;
        // Moving to a descendant of the same tip element isn't a real
        // "leave" — mouseout/mouseover fire on every inner boundary
        // crossing too (they bubble, unlike mouseenter/mouseleave).
        const related = e.relatedTarget;
        if (related instanceof Node && el.contains(related)) return;
        setActiveEl(null);
    };
    // `:focus-visible` parity with the CSS this replaces — keyboard-tabbing
    // onto a status-bar control shows its tip too, not just hover.
    const onFocusIn = (e: FocusEvent) => {
        const el = findTipEl(e.target);
        if (el && el.matches(":focus-visible")) setActiveEl(el);
    };
    const onFocusOut = (e: FocusEvent) => {
        const el = findTipEl(e.target);
        if (el && el === activeEl()) setActiveEl(null);
    };

    document.addEventListener("mouseover", onMouseOver);
    document.addEventListener("mouseout", onMouseOut);
    document.addEventListener("focusin", onFocusIn);
    document.addEventListener("focusout", onFocusOut);
    onCleanup(() => {
        document.removeEventListener("mouseover", onMouseOver);
        document.removeEventListener("mouseout", onMouseOut);
        document.removeEventListener("focusin", onFocusIn);
        document.removeEventListener("focusout", onFocusOut);
    });

    // `keyed` so the element is captured as a plain value, not re-read
    // reactively. Without it, `target` stayed bound to `activeEl()` for the
    // balloon's whole life: any position update still in flight when the tip
    // was dismissed read the freshly-nulled signal and threw
    // `Cannot read properties of null (reading 'getBoundingClientRect')`.
    // `keyed` also re-creates the balloon when the cursor moves straight
    // from one `[data-tip]` to another, which is what we want anyway — a
    // new anchor needs its own `autoUpdate`, not a mutated reference.
    return (
        <Show when={activeEl()} keyed>
            {(el) => <TipBalloon target={el} text={el.getAttribute("data-tip") ?? ""} />}
        </Show>
    );
};

StatusBarTip.displayName = "StatusBarTip";
