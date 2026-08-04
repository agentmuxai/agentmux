// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Menu positioning framework — Phase 1 primitive.
//
// Implements SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20 §5: one hook every
// DOM menu routes through so menus offset away from BOTH window edges and
// native browser-pane child windows (which paint above the host webview's
// DOM and would otherwise clip/occlude a menu placed "inside the window").
//
// Phase 1 ships the primitive only — nothing calls it yet except the unit
// tests. Migration of FlyoutMenu/Popover/etc. is Phase 2-3.

import {
    computePosition,
    flip,
    offset,
    shift,
    size,
    type Placement,
} from "@floating-ui/dom";
import type { JSX } from "solid-js";

// ── Public types (spec §5) ──────────────────────────────────────────────────

export interface MenuPositionRequest {
    /** Element rect, a raw DOMRect, or a click point. */
    anchor: HTMLElement | DOMRect | { x: number; y: number };
    /** Preferred side; flips to the opposite side of the anchor if it doesn't fit. */
    placement?: Placement;
    /** Minimum gap from the anchor and from any paintable edge. Default 8. */
    gutter?: number;
    /**
     * Cross-axis nudge (px) — shifts the menu perpendicular to its placement
     * side, exactly like Floating UI's `offset({ crossAxis })`. Default 0.
     */
    offsetCrossAxis?: number;
    /**
     * Alignment-axis nudge (px) — overrides {@link offsetCrossAxis} for aligned
     * (`*-start` / `*-end`) placements, like Floating UI's
     * `offset({ alignmentAxis })`. Default: unset (crossAxis applies).
     */
    offsetAlignmentAxis?: number;
    /** Treat native browser-pane rects as boundaries. Default true. */
    avoidNativePanes?: boolean;
}

export interface MenuPositionResult {
    /** position:fixed + left/top — the computed offset, applied. */
    style: JSX.CSSProperties;
    /** Side chosen after any flip. */
    placement: Placement;
    /** Cap when the menu is taller than the free space (internal scroll). */
    maxHeight: number;
    /** Cap when the menu is wider than the free space. */
    maxWidth: number;
}

const DEFAULT_GUTTER = 8;

// ── Native-pane rectangles (spec §5.3) ───────────────────────────────────────

/**
 * Rectangles of native child windows that paint ABOVE the host webview's DOM.
 *
 * Browser panes are `CefBrowserView` instances — native OS child windows, not
 * iframes — so a DOM menu overlapping one is drawn behind it. Editor panes are
 * CodeMirror rendered into a DOM `<div>` inside the webview (verified against
 * `frontend/app/view/editor/editor-view.tsx`: `new EditorView({...})` with
 * `@codemirror/*` extensions), so they do NOT occlude DOM menus and are NOT
 * included here.
 *
 * Geometry source: the layout reducer tracks per-block node *sizes*, but not
 * directly the on-screen client rects in DOM-pixel space. The browser pane's
 * own resize path (`browser-view.tsx`) derives its rect from a stable
 * `.browser-placeholder` element via `getBoundingClientRect()`. We mirror that:
 * query every `.browser-placeholder` in the document and measure it. This is
 * the exact rect the host already syncs to the native HWND, so it is the
 * authoritative on-screen geometry. Measured once per menu-open (cheap — a
 * tab rarely has more than ~6 panes; spec §10 risk row 5).
 */
export function getNativePaneRects(): DOMRect[] {
    if (typeof document === "undefined") return [];
    const rects: DOMRect[] = [];
    // `.browser-placeholder` is the wrapper div whose getBoundingClientRect()
    // browser-view.tsx feeds to `browser_pane_resize` — i.e. the live pane rect.
    const panes = document.querySelectorAll<HTMLElement>(".browser-placeholder");
    panes.forEach((el) => {
        const r = el.getBoundingClientRect();
        // Skip zero-area placeholders (pane not yet created / detached tab).
        if (r.width > 0 && r.height > 0) rects.push(r);
    });
    return rects;
}

// ── Paintable area (spec §5.1 step 3, §10 risk row 1) ────────────────────────

function makeRect(x: number, y: number, width: number, height: number): DOMRect {
    // DOMRect.fromRect isn't in every jsdom build — construct a plain rect.
    return {
        x,
        y,
        width,
        height,
        top: y,
        left: x,
        right: x + width,
        bottom: y + height,
        toJSON() {
            return { x, y, width, height };
        },
    } as DOMRect;
}

function intersects(a: DOMRect, b: DOMRect): boolean {
    return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}

function area(r: DOMRect): number {
    return Math.max(0, r.width) * Math.max(0, r.height);
}

/**
 * The window viewport minus native-pane rects, plus the largest inscribed
 * axis-aligned free rectangle.
 *
 * floating-ui's `boundary` accepts a single rect, but "viewport minus N panes"
 * is generally not one rect (spec §10 risk row 1). Heuristic for v1: take the
 * bounding union of all panes, then test the four viewport sub-regions around
 * that union (the strips above / below / left / right of the union) plus the
 * full viewport when no pane intersects it; pick the largest free one. This is
 * an approximation — it does not find the true maximal empty rectangle — but it
 * is correct for the common AgentMux layouts (panes tiled against an edge) and
 * cheap. The §5.1-step-5 residual check (Phase 2+) catches leftover overlap.
 */
export function getPaintableArea(): { rects: DOMRect[]; largestFreeRect: DOMRect } {
    const vw = typeof window !== "undefined" ? window.innerWidth : 0;
    const vh = typeof window !== "undefined" ? window.innerHeight : 0;
    const viewport = makeRect(0, 0, vw, vh);
    const rects = getNativePaneRects();

    // No panes — the whole viewport is paintable.
    if (rects.length === 0) {
        return { rects, largestFreeRect: viewport };
    }

    // Bounding union of every pane rect, clamped to the viewport.
    let uL = Infinity, uT = Infinity, uR = -Infinity, uB = -Infinity;
    for (const r of rects) {
        uL = Math.min(uL, r.left);
        uT = Math.min(uT, r.top);
        uR = Math.max(uR, r.right);
        uB = Math.max(uB, r.bottom);
    }
    uL = Math.max(0, uL);
    uT = Math.max(0, uT);
    uR = Math.min(vw, uR);
    uB = Math.min(vh, uB);

    // Candidate free strips around the pane union.
    const candidates: DOMRect[] = [
        makeRect(0, 0, vw, uT), // strip above the union
        makeRect(0, uB, vw, vh - uB), // strip below the union
        makeRect(0, 0, uL, vh), // strip left of the union
        makeRect(uR, 0, vw - uR, vh), // strip right of the union
    ];

    // Keep candidates that are non-degenerate and don't intersect any pane.
    let best = candidates[0];
    let bestArea = -1;
    for (const c of candidates) {
        if (c.width <= 0 || c.height <= 0) continue;
        if (rects.some((p) => intersects(c, p))) continue;
        const a = area(c);
        if (a > bestArea) {
            bestArea = a;
            best = c;
        }
    }

    // Fallback: panes cover every strip (e.g. a centered pane). Use the
    // viewport itself — Phase 2's residual check / shrink handles the rest.
    if (bestArea < 0) {
        return { rects, largestFreeRect: viewport };
    }
    return { rects, largestFreeRect: best };
}

// ── Anchor resolution (spec §5.1 step 1) ─────────────────────────────────────

function isPoint(a: MenuPositionRequest["anchor"]): a is { x: number; y: number } {
    return (
        typeof (a as { x?: unknown }).x === "number" &&
        typeof (a as { y?: unknown }).y === "number" &&
        typeof (a as { width?: unknown }).width !== "number"
    );
}

/** Resolve any anchor form to a DOMRect. Point → zero-size rect at x/y. */
function resolveAnchorRect(anchor: MenuPositionRequest["anchor"]): DOMRect {
    if (anchor instanceof HTMLElement) {
        return anchor.getBoundingClientRect();
    }
    if (isPoint(anchor)) {
        return makeRect(anchor.x, anchor.y, 0, 0);
    }
    return anchor as DOMRect;
}

// ── Core computation — exported for direct/test use ─────────────────────────

/** floating-ui accepts a virtual element exposing getBoundingClientRect. */
function virtualReference(rect: DOMRect): { getBoundingClientRect: () => DOMRect } {
    return { getBoundingClientRect: () => rect };
}

/**
 * Run the fixed floating-ui middleware stack (spec §5.1 step 2):
 * offset({mainAxis:gutter, crossAxis, alignmentAxis}) → flip() →
 * shift({padding:gutter}) → size().
 *
 * When `avoidNativePanes` is true, every overflow-detecting middleware uses the
 * paintable area's largest free rect as `boundary` — this is the line that
 * makes flip/shift offset away from native panes, not just window edges.
 *
 * Async because computePosition is async; callers (and tests) await it.
 */
export async function computeMenuPosition(
    request: MenuPositionRequest,
    floatingEl: HTMLElement,
): Promise<MenuPositionResult> {
    const gutter = request.gutter ?? DEFAULT_GUTTER;
    const avoidNativePanes = request.avoidNativePanes ?? true;
    const placement = request.placement ?? "bottom-start";

    const anchorRect = resolveAnchorRect(request.anchor);
    const reference =
        request.anchor instanceof HTMLElement
            ? request.anchor
            : virtualReference(anchorRect);

    // boundary: the paintable area's largest free rect when avoiding panes;
    // otherwise the raw window viewport. We pass an explicit Rect in both
    // cases (rather than leaning on floating-ui's default "clippingAncestors")
    // so overflow detection is deterministic — clippingAncestors walks the DOM
    // and is unreliable for a portal'd, not-yet-laid-out menu. A plain Rect is
    // a valid floating-ui Boundary.
    const viewportRect = makeRect(
        0,
        0,
        typeof window !== "undefined" ? window.innerWidth : 0,
        typeof window !== "undefined" ? window.innerHeight : 0,
    );
    const boundary = avoidNativePanes
        ? getPaintableArea().largestFreeRect
        : viewportRect;
    const overflowOpts = { boundary: { ...boundary } as DOMRect, padding: gutter };

    let maxHeight = typeof window !== "undefined" ? window.innerHeight : 0;
    let maxWidth = typeof window !== "undefined" ? window.innerWidth : 0;

    const computed = await computePosition(reference as Element, floatingEl, {
        strategy: "fixed",
        placement,
        middleware: [
            offset({
                mainAxis: gutter,
                crossAxis: request.offsetCrossAxis ?? 0,
                alignmentAxis: request.offsetAlignmentAxis,
            }),
            flip({ ...overflowOpts }),
            shift({ ...overflowOpts }),
            size({
                ...overflowOpts,
                apply({ availableWidth, availableHeight }) {
                    // Capture the free space so a too-tall/wide menu can scroll
                    // internally instead of being placed partly outside (§5.2
                    // step 1 — shrink).
                    maxHeight = Math.max(0, Math.floor(availableHeight));
                    maxWidth = Math.max(0, Math.floor(availableWidth));
                },
            }),
        ],
    });

    return {
        style: {
            position: "fixed",
            left: `${Math.round(computed.x)}px`,
            top: `${Math.round(computed.y)}px`,
        },
        placement: computed.placement,
        maxHeight,
        maxWidth,
    };
}

// ── Dev-mode guard (spec §6.1) ───────────────────────────────────────────────

/** True only in dev builds. Vite static-replaces `import.meta.env.DEV`, so the
 *  whole guard body is dead-code-eliminated from release bundles. */
function isDevBuild(): boolean {
    try {
        return import.meta.env.DEV === true;
    } catch {
        return false;
    }
}

/** Elements that have already logged a violation this open — debounce so the
 *  guard logs at most once per element per open (spec §10 risk row 4). A
 *  WeakSet keeps no element alive past unmount. */
const guardedElements = new WeakSet<HTMLElement>();

/**
 * Dev-only runtime assertion (spec §6.1): one RAF after a menu opens, measure
 * its rect and confirm the whole body lands inside the paintable area — i.e.
 * within the window and not behind a native browser-pane child window. Any
 * violation is a `console.error` tagged `[menu-guard]` (surfaces via
 * `muxlog host '[menu-guard]'`).
 *
 * Zero-cost in release builds: the `isDevBuild()` gate is static-replaced to
 * `false` by Vite and the whole body is dropped. Callers still gate the call
 * site too so the RAF schedule itself is elided.
 *
 * @param el    the menu's floating DOM node
 * @param label short identifier for the menu surface, e.g. "flyout-menu"
 */
export function assertMenuInPaintableArea(el: HTMLElement, label: string): void {
    if (!isDevBuild()) return;
    if (typeof requestAnimationFrame === "undefined") return;
    requestAnimationFrame(() => {
        if (!el.isConnected) return;
        if (guardedElements.has(el)) return;

        const rect = el.getBoundingClientRect();
        // A not-yet-laid-out menu reports a zero rect — skip, RAF was too early.
        if (rect.width <= 0 || rect.height <= 0) return;

        const vw = typeof window !== "undefined" ? window.innerWidth : 0;
        const vh = typeof window !== "undefined" ? window.innerHeight : 0;

        const violations: string[] = [];

        // 1. Window edges.
        if (rect.left < 0) violations.push(`left edge (left=${Math.round(rect.left)})`);
        if (rect.top < 0) violations.push(`top edge (top=${Math.round(rect.top)})`);
        if (rect.right > vw) {
            violations.push(`right edge (right=${Math.round(rect.right)} > ${vw})`);
        }
        if (rect.bottom > vh) {
            violations.push(`bottom edge (bottom=${Math.round(rect.bottom)} > ${vh})`);
        }

        // 2. Native-pane rects — a menu overlapping one is drawn behind it.
        for (const pane of getNativePaneRects()) {
            if (intersects(rect, pane)) {
                violations.push(
                    `behind native pane (pane ${Math.round(pane.left)},` +
                        `${Math.round(pane.top)} ${Math.round(pane.width)}x` +
                        `${Math.round(pane.height)})`,
                );
            }
        }

        if (violations.length > 0) {
            guardedElements.add(el);
            // eslint-disable-next-line no-console
            console.error(
                `[menu-guard] "${label}" rendered outside the paintable area: ` +
                    violations.join("; ") +
                    ` — menu rect ${Math.round(rect.left)},${Math.round(rect.top)} ` +
                    `${Math.round(rect.width)}x${Math.round(rect.height)}`,
            );
        }
    });
}
