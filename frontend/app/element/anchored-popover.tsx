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
 *   - The colours of the pane it was opened from, when that pane has a backdrop (`carryPaneScope`): the shell
 *     carries the pane's scope for the overlay layer. The pane is the one the anchor is in, or `scope`.
 *   - Dismiss, when `onDismiss` is given: a mousedown outside the panel and
 *     its anchor, or Esc (not while typing in a field inside the panel).
 *   - Entrance and exit, when `motion` is given: the panel stays hidden until
 *     its first placement, then plays `enter`; when it unmounts, a
 *     non-interactive copy of the shell plays `exit` and removes itself (see
 *     `fadeOutCopy`), so the owner's `<Show>` still drops the real panel at once.
 *
 * Mount it only while the panel is open (inside a `<Show>`); its lifetime is
 * the panel's.
 */

import { carryPaneScope, type ScopeSource } from "@/app/backdrop/overlay-scope";
import { fadeOutCopy } from "@/app/element/exit-copy";
import type { EnterKind } from "@/app/motion/primitives";
import { useMotion } from "@/app/motion/solid";
import { useHiddenTabMarker, usePaneOverlay } from "@/app/platform/pane-overlay";
import { holdBottomEdge } from "@/app/util/hold-bottom-edge";
import { computeMenuPosition } from "@/app/util/menu-position";
import { autoUpdate, type Placement } from "@floating-ui/dom";
import { createSignal, onCleanup, onMount, untrack, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import "./anchored-popover.scss";
import { setOverlayAnchor } from "@/app/capture/overlay-anchors";

/** A zero-size rect at a point: the cursor, for a menu that opens where the user clicked. */
export function pointAnchor(x: number, y: number): DOMRect {
    return { x, y, left: x, top: y, right: x, bottom: y, width: 0, height: 0, toJSON: () => ({}) } as DOMRect;
}

/** An element (followed live) or a fixed rect. */
export type PopoverAnchor = Element | DOMRect | null | undefined;

export interface AnchoredPopoverProps {
    /** Read once, when the panel's first frame runs. */
    anchor: PopoverAnchor;
    placement?: Placement;
    /** The pane whose backdrop the panel takes: an element in it, or its block id. Default: the anchor, when it is an element. */
    scope?: ScopeSource;
    /** The gap to the anchor, in real pixels. Default 8. */
    gutter?: number;
    /** How the panel comes in and goes out (`enter`/`exit` kind). Omit for none. */
    motion?: EnterKind;
    /** Keep the panel hidden until its first placement, so it never shows at the corner (implied by `motion`). */
    hideUntilPlaced?: boolean;
    /** Forward keys to the panel while a native browser pane has focus. Default: when `onDismiss` is given. */
    overlayKeys?: boolean;
    /** Once, when the panel is first shown in place. With `motion` that is after its first placement, not at mount. */
    onShown?: () => void;
    /** `"chrome"` (default): the body scales with chrome zoom (`--zoomfactor`). */
    zoom?: "chrome" | "none";
    /** Called on an outside mousedown or Esc, with which it was. Omit for a panel that dismisses itself (a hover tip). */
    onDismiss?: (reason: "outside" | "escape") => void;
    /** A press on this target counts as inside (a submenu, mounted apart from its parent). */
    keepOpenOn?: (target: Node) => boolean;
    /** Keep the panel inside this element (a browser pane) when it fits; with `alignToAnchorSide`, open it from the side its anchor is on. */
    containWithin?: () => HTMLElement | null | undefined;
    alignToAnchorSide?: boolean;
    /** Cap the body at the room the window leaves and scroll inside it, instead of running off the window. */
    fitHeight?: boolean;
    /** With `containWithin`: tell the body how wide its container is, as `--popover-room-width` (in the body's own zoomed pixels), so its style can keep it inside a narrow pane. */
    fitWidth?: boolean;
    /** Classes for the body: the panel's own styling. */
    class?: string;
    /** Inline style for the body (e.g. a fixed width, which then scales too). */
    style?: JSX.CSSProperties;
    /** The panel's controls take input from the person only: the shell is marked `data-operator-only`, which the UI tool's click, query and
     *  focus refuse and which holds keys and commands back while it is open (crates/cef/src/browser_api/scripts/query.js). */
    operatorOnly?: boolean;
    /** Classes for the shell: stacking (z-index) and pointer-events only. */
    shellClass?: string;
    role?: JSX.HTMLAttributes<HTMLDivElement>["role"];
    "aria-label"?: string;
    /** The body element. */
    ref?: (el: HTMLDivElement) => void;
    children?: JSX.Element;
}

const IDLE_STYLE: JSX.CSSProperties = { position: "fixed", left: "0px", top: "0px" };
// A panel that animates in waits, unseen, for its first placement, so it does not fade in at the corner.
const WAITING_STYLE: JSX.CSSProperties = { ...IDLE_STYLE, visibility: "hidden" };

const ROOM_GUTTER = 8;
const MIN_ROOM_HEIGHT = 96;

/** The room a panel is asked to fit in, in screen pixels: the height the window leaves, and the width of its container. */
interface Room {
    height?: number;
    width?: number;
}

/** `anchorTop`: where the anchor is, for a panel opening upward. With `fitWidth` the panel also stays under its container's top edge. */
function roomFor(maxHeight: number, container: HTMLElement | undefined, fitHeight?: boolean, fitWidth?: boolean, anchorTop?: number): Room {
    const own = fitHeight && Number.isFinite(maxHeight) ? maxHeight : undefined;
    const underTop = own !== undefined && fitWidth && container && anchorTop !== undefined ? anchorTop - container.getBoundingClientRect().top - ROOM_GUTTER : undefined;
    return {
        height: underTop === undefined ? own : Math.max(MIN_ROOM_HEIGHT, Math.min(own!, underTop)),
        width: fitWidth && container ? Math.floor(Math.min(container.getBoundingClientRect().width, window.innerWidth)) : undefined,
    };
}

/** The body's own style with its room: the room is in screen pixels and the body is zoomed, so each is divided by the zoom. */
function roomStyle(base: JSX.CSSProperties | undefined, room: Room): JSX.CSSProperties | undefined {
    if (room.height === undefined && room.width === undefined) return base;
    return {
        ...base,
        ...(room.height === undefined ? {} : { "max-height": `calc(${room.height}px / var(--zoomfactor, 1))`, "overflow-y": "auto" }),
        ...(room.width === undefined ? {} : { "--popover-room-width": `calc(${room.width}px / var(--zoomfactor, 1))` }),
    };
}

/** A floating-ui reference that keeps reading the anchor captured at mount. */
/** What the panel is placed against; also records it as the panel's anchor (a window capture's mask reads it). */
function referenceFor(anchor: Element | DOMRect, panel?: Element) {
    if (panel) setOverlayAnchor(panel, anchor);
    if (anchor instanceof Element) {
        return { getBoundingClientRect: () => anchor.getBoundingClientRect(), contextElement: anchor };
    }
    return { getBoundingClientRect: () => anchor };
}

/** The body's classes, as one string. */
function bodyClass(props: AnchoredPopoverProps): string {
    const chrome = (props.zoom ?? "chrome") === "chrome" ? "anchored-popover-body--chrome-zoom" : "";
    return ["anchored-popover-body", chrome, props.class ?? ""].filter(Boolean).join(" ");
}

function isEditable(el: EventTarget | null): boolean {
    if (!(el instanceof HTMLElement)) return false;
    return el.isContentEditable || el.matches("input, textarea, select");
}

// The dismissable popovers open now, so a surface that takes the window (the command palette) can close them first.
const openDismissers = new Set<() => void>();

/** Dismisses every open popover that can be dismissed (a menu, a list from a control), as a press outside would. */
export function dismissAnchoredPopovers(): void {
    for (const dismiss of [...openDismissers]) dismiss();
}

/** Dismiss on a press outside the panel (its anchor and what `keepOpenOn` names count as inside) or on Esc. */
function listenForDismiss(props: AnchoredPopoverProps, shell: () => HTMLElement | undefined): void {
    if (!props.onDismiss) return;
    const dismiss = () => props.onDismiss?.("outside");
    openDismissers.add(dismiss);
    onCleanup(() => openDismissers.delete(dismiss));
    const anchorEl = (): Element | null => {
        const a = untrack(() => props.anchor);
        return a instanceof Element ? a : null;
    };
    const onMouseDown = (e: MouseEvent) => {
        const t = e.target as Node;
        if (shell()?.contains(t) || anchorEl()?.contains(t) || props.keepOpenOn?.(t)) return;
        props.onDismiss?.("outside");
    };
    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key !== "Escape") return;
        // Esc in a field inside the panel belongs to the field (e.g. cancel a rename).
        if (isEditable(e.target) && shell()?.contains(e.target as Node)) return;
        e.stopPropagation();
        props.onDismiss?.("escape");
    };
    document.addEventListener("mousedown", onMouseDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    onCleanup(() => {
        document.removeEventListener("mousedown", onMouseDown, true);
        window.removeEventListener("keydown", onKeyDown, true);
    });
}

/**
 * Keeps the shell carrying the scope of the pane the panel was opened from: `scope`, or the pane its anchor is in.
 * Asked once the panel is mounted, which is before its first paint and after the owner's own refs are set. Returns
 * the function that takes the scope off again.
 */
function usePaneScope(props: AnchoredPopoverProps, shell: () => HTMLElement | undefined): () => void {
    let stop: (() => void) | undefined;
    onMount(() => {
        const el = shell();
        if (el) stop = carryPaneScope(el, untrack(() => props.scope) ?? untrack(() => props.anchor));
    });
    return () => stop?.();
}

export const AnchoredPopover = (props: AnchoredPopoverProps): JSX.Element => {
    // Portal-mounted: drop its pane hole while its window tab is hidden (pane-overlay.ts).
    const hiddenTab = useHiddenTabMarker();
    const motion = useMotion();
    let shell: HTMLDivElement | undefined;
    let body: HTMLDivElement | undefined;
    let entered = false;
    const [room, setRoom] = createSignal<Room>({});
    const [shellStyle, setShellStyle] = createSignal<JSX.CSSProperties>(untrack(() => props.motion || props.hideUntilPlaced) ? WAITING_STYLE : IDLE_STYLE);
    /** The first time the panel is shown, in place: play its entrance. */
    const shown = () => {
        if (entered) return;
        entered = true;
        const kind = untrack(() => props.motion);
        if (kind && body) motion.enter(body, kind);
        untrack(() => props.onShown)?.();
    };
    let stopAutoUpdate: (() => void) | null = null;
    let frame: number | undefined;
    let disposed = false;
    const stopScope = usePaneScope(props, () => shell);
    // A dismissible popover takes Escape; a passive one (a tooltip) does not.
    usePaneOverlay(() => shell, { keys: props.overlayKeys ?? !!props.onDismiss });

    const registerShell = (el: HTMLDivElement) => {
        shell = el;
        if (frame !== undefined) cancelAnimationFrame(frame);
        frame = requestAnimationFrame(() => {
            frame = undefined;
            const anchor = untrack(() => props.anchor);
            if (!anchor || !(el instanceof Element)) {
                // Nothing to place against: show it where it is.
                setShellStyle(IDLE_STYLE);
                shown();
                return;
            }
            const reference = referenceFor(anchor, el);
            const placement = untrack(() => props.placement) ?? "top-start";
            const container = untrack(() => props.containWithin)?.() ?? undefined;
            const update = async () => {
                if (disposed) return;
                const pos = await computeMenuPosition(
                    {
                        anchor: reference.getBoundingClientRect(),
                        placement,
                        gutter: untrack(() => props.gutter),
                        avoidNativePanes: false,
                        containWithin: container,
                        alignToAnchorSide: untrack(() => props.alignToAnchorSide),
                    },
                    el,
                );
                if (disposed) return;
                setShellStyle(holdBottomEdge(pos, el.getBoundingClientRect().height, window.innerHeight));
                setRoom(roomFor(pos.maxHeight, container, untrack(() => props.fitHeight), untrack(() => props.fitWidth), placement.startsWith("top") ? reference.getBoundingClientRect().top : undefined));
                shown();
            };
            stopAutoUpdate?.();
            const stopFloating = autoUpdate(reference, el, update);
            // autoUpdate watches the anchor and the panel, not the container: a pane resized under an open panel moves it.
            const observer = container && typeof ResizeObserver !== "undefined" ? new ResizeObserver(() => void update()) : undefined;
            if (container) observer?.observe(container);
            stopAutoUpdate = () => {
                stopFloating();
                observer?.disconnect();
            };
        });
    };

    onMount(() => listenForDismiss(props, () => shell));

    onCleanup(() => {
        disposed = true;
        const kind = untrack(() => props.motion);
        if (kind && shell && entered) fadeOutCopy(shell, kind);
        stopScope(); // after the copy is made: the copy that fades out keeps the pane's colours
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
                data-pane-overlay data-operator-only={props.operatorOnly ? "" : undefined}
                data-window-tab-hidden={hiddenTab()}
                data-zoom-scope={(props.zoom ?? "chrome") === "chrome" ? "chrome" : undefined}
            >
                <div
                    ref={(el) => {
                        body = el;
                        props.ref?.(el);
                    }}
                    // One class string, not class + classList: a changing
                    // `class` would wipe what classList added.
                    class={bodyClass(props)}
                    // The room is in screen pixels and the body is zoomed, so its own max-height is the room over the zoom.
                    style={roomStyle(props.style, room())}
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
