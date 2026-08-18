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

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { autoUpdate } from "@floating-ui/dom";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { computeMenuPosition } from "@/app/util/menu-position";
import "./StatusBarTip.scss";

interface TipBalloonProps {
    target: HTMLElement;
    text: string;
}

/**
 * The balloon itself, split out so `usePaneOverlay` and the floating-ui
 * position registration run against ITS OWN mount lifecycle (only while a
 * tip is showing) — same reasoning as `HostPopoverPanel`/
 * `BackendStatusPanel`/`TokenBreakdownPopover`: calling `usePaneOverlay`
 * in the always-mounted parent would read an undefined ref at the parent's
 * mount time and never re-attach its observers once the ref is later set.
 */
const TipBalloon = (props: TipBalloonProps): JSX.Element => {
    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
    });
    let cleanupAutoUpdate: (() => void) | null = null;
    let rootRef: HTMLDivElement | undefined;

    // Airspace cut so the balloon paints over any browser-pane HWND the
    // status bar overlaps — same primitive as the status-bar popovers.
    usePaneOverlay(() => rootRef);

    const registerFloating = (el: HTMLDivElement) => {
        rootRef = el;
        requestAnimationFrame(() => {
            if (!(el instanceof Element)) return;
            const update = async () => {
                const pos = await computeMenuPosition(
                    { anchor: props.target.getBoundingClientRect(), placement: "top", avoidNativePanes: false },
                    el,
                );
                setFloatingStyle(pos.style);
            };
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(
                { getBoundingClientRect: () => props.target.getBoundingClientRect() },
                el,
                update,
            );
        });
    };

    onCleanup(() => cleanupAutoUpdate?.());

    return (
        <div
            ref={registerFloating}
            class="status-bar-tip-balloon"
            data-pane-overlay
            style={floatingStyle()}
        >
            {props.text}
        </div>
    );
};

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

    return (
        <Show when={activeEl()}>
            <Portal>
                <TipBalloon target={activeEl()!} text={activeEl()!.getAttribute("data-tip") ?? ""} />
            </Portal>
        </Show>
    );
};

StatusBarTip.displayName = "StatusBarTip";
