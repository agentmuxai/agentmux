// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AnchoredPopover — the one floating panel every status bar popover, the
 * status bar tips, the version panel and the agent pane's model/effort and
 * session panels render through. It owns what each of them used to copy:
 *
 *   - Portal to `document.body`.
 *   - A shell / body pair, so the panel follows chrome zoom. The shell is
 *     positioned in real pixels by `computeMenuPosition` and has no zoom of
 *     its own; the body inside it carries `zoom: var(--zoomfactor)` and every
 *     visual style. The shell shrink-wraps the zoomed body, so floating-ui
 *     measures the true on-screen size. Zooming the positioned element itself
 *     applies the factor twice (#2736), and dividing its position by the zoom
 *     still misplaces it, because floating-ui reads its size in CSS pixels
 *     (both measured in Chrome 154: docs/reports/REPORT_CHROME_ZOOM_POPOVERS_2026_10_02.md §4).
 *   - Positioning: `autoUpdate` starts one frame after mount and stops on
 *     cleanup, including a frame that has not run yet (the leak StatusBarTip
 *     fixed, now fixed once for all).
 *   - The airspace cut over native browser panes (`usePaneOverlay` plus
 *     `data-pane-overlay`), on every platform.
 *   - Dismiss, when `onDismiss` is given: a mousedown outside the panel and
 *     its anchor, or Esc (not while typing in a field inside the panel).
 *
 * Mount it only while the panel is open (inside a `<Show>`); its lifetime is
 * the panel's.
 */

import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { computeMenuPosition } from "@/app/util/menu-position";
import { autoUpdate, type Placement } from "@floating-ui/dom";
import { createSignal, onCleanup, onMount, untrack, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import "./anchored-popover.scss";

/** An element (followed live) or a fixed rect. */
export type PopoverAnchor = Element | DOMRect | null | undefined;

export interface AnchoredPopoverProps {
    /** Read once, when the panel's first frame runs. */
    anchor: PopoverAnchor;
    placement?: Placement;
    /** `"chrome"` (default): the body scales with chrome zoom (`--zoomfactor`). */
    zoom?: "chrome" | "none";
    /** Called on an outside mousedown or Esc. Omit for a panel that dismisses itself (a hover tip). */
    onDismiss?: () => void;
    /** Classes for the body: the panel's own styling. */
    class?: string;
    /** Inline style for the body (e.g. a fixed width, which then scales too). */
    style?: JSX.CSSProperties;
    /** Classes for the shell: stacking (z-index) and pointer-events only. */
    shellClass?: string;
    role?: JSX.HTMLAttributes<HTMLDivElement>["role"];
    "aria-label"?: string;
    /** The body element. */
    ref?: (el: HTMLDivElement) => void;
    children?: JSX.Element;
}

const IDLE_STYLE: JSX.CSSProperties = { position: "fixed", left: "0px", top: "0px" };

/** A floating-ui reference that keeps reading the anchor captured at mount. */
function referenceFor(anchor: Element | DOMRect) {
    if (anchor instanceof Element) {
        return { getBoundingClientRect: () => anchor.getBoundingClientRect(), contextElement: anchor };
    }
    return { getBoundingClientRect: () => anchor };
}

function isEditable(el: EventTarget | null): boolean {
    if (!(el instanceof HTMLElement)) return false;
    return el.isContentEditable || el.matches("input, textarea, select");
}

export const AnchoredPopover = (props: AnchoredPopoverProps): JSX.Element => {
    let shell: HTMLDivElement | undefined;
    const [shellStyle, setShellStyle] = createSignal<JSX.CSSProperties>(IDLE_STYLE);
    let stopAutoUpdate: (() => void) | null = null;
    let frame: number | undefined;
    let disposed = false;

    usePaneOverlay(() => shell);

    const registerShell = (el: HTMLDivElement) => {
        shell = el;
        if (frame !== undefined) cancelAnimationFrame(frame);
        frame = requestAnimationFrame(() => {
            frame = undefined;
            const anchor = untrack(() => props.anchor);
            if (!anchor || !(el instanceof Element)) return;
            const reference = referenceFor(anchor);
            const placement = untrack(() => props.placement) ?? "top-start";
            const update = async () => {
                const pos = await computeMenuPosition(
                    { anchor: reference.getBoundingClientRect(), placement, avoidNativePanes: false },
                    el,
                );
                if (!disposed) setShellStyle(pos.style);
            };
            stopAutoUpdate?.();
            stopAutoUpdate = autoUpdate(reference, el, update);
        });
    };

    onMount(() => {
        if (!props.onDismiss) return;
        const anchorEl = (): Element | null => {
            const a = untrack(() => props.anchor);
            return a instanceof Element ? a : null;
        };
        const onMouseDown = (e: MouseEvent) => {
            const t = e.target as Node;
            if (shell?.contains(t) || anchorEl()?.contains(t)) return;
            props.onDismiss?.();
        };
        const onKeyDown = (e: KeyboardEvent) => {
            if (e.key !== "Escape") return;
            // Esc in a field inside the panel belongs to the field (e.g. cancel a rename).
            if (isEditable(e.target) && shell?.contains(e.target as Node)) return;
            e.stopPropagation();
            props.onDismiss?.();
        };
        document.addEventListener("mousedown", onMouseDown, true);
        window.addEventListener("keydown", onKeyDown, true);
        onCleanup(() => {
            document.removeEventListener("mousedown", onMouseDown, true);
            window.removeEventListener("keydown", onKeyDown, true);
        });
    });

    onCleanup(() => {
        disposed = true;
        if (frame !== undefined) cancelAnimationFrame(frame);
        frame = undefined;
        stopAutoUpdate?.();
        stopAutoUpdate = null;
    });

    return (
        <Portal>
            <div
                ref={registerShell}
                class={`anchored-popover${props.shellClass ? ` ${props.shellClass}` : ""}`}
                style={shellStyle()}
                data-pane-overlay
                data-zoom-scope={(props.zoom ?? "chrome") === "chrome" ? "chrome" : undefined}
            >
                <div
                    ref={props.ref}
                    // One class string, not class + classList: a changing
                    // `class` would wipe what classList added.
                    class={[
                        "anchored-popover-body",
                        (props.zoom ?? "chrome") === "chrome" ? "anchored-popover-body--chrome-zoom" : "",
                        props.class ?? "",
                    ].filter(Boolean).join(" ")}
                    style={props.style}
                    role={props.role}
                    aria-label={props["aria-label"]}
                >
                    {props.children}
                </div>
            </div>
        </Portal>
    );
};

AnchoredPopover.displayName = "AnchoredPopover";
