// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, onCleanup, onMount, type Accessor, type JSX, type Setter } from "solid-js";
import { getApi } from "@/util/app-api";
import { hidesPaneUnderOverlay, watchPaneClip } from "@/app/platform/pane-backend";
import type { BrowserViewModel } from "./browser-model";
import type { PaneRect } from "./use-pane-rect-sync";

export interface FreezeFrame {
    freezeSnapshot: () => string | null;
    freezeStyle: () => JSX.CSSProperties;
    onFreezePrewarm: () => void;
    onOverlayClipChanged: (e: Event) => void;
}

// ── Freeze-frame while airspace-hidden ─────────────────────────────
//
// Only for a pane the host hides whole while a DOM overlay (AgentMux logo menu,
// modal, …) is over it (`hidesPaneUnderOverlay`: a Views pane on Linux or
// Windows with the host's clip turned off). Hiding it exposes the bare
// placeholder and reads as the pane "going black". Instead, capture the pane's
// last rendered frame (CDP `Page.captureScreenshot` via the browser DOM API;
// the compositor keeps producing frames while the pane is hidden, verified
// empirically) and display it in the placeholder until the overlay clears. The
// frame is static, which is acceptable for menu-open-scale durations.
//
// Everywhere else the host cuts a hole in the pane and the pane stays live, so
// freezing would be harmful: it forces a compositor frame, visible as a jerk,
// and defers the clip IPC.

// Offsets from the placeholder's edges, not a fixed size: the frame then
// follows the placeholder while a splitter is dragged, stretched in step with
// the chrome around it. Aligned to the native pane's DPR-rounded rect.
function freezeGeometry(box: DOMRect, pr: PaneRect, dpr: number): JSX.CSSProperties {
    const left = pr.x / dpr - box.x;
    const top = pr.y / dpr - box.y;
    return {
        left: `${left}px`,
        top: `${top}px`,
        width: `calc(100% - ${box.width - pr.width / dpr}px)`,
        height: `calc(100% - ${box.height - pr.height / dpr}px)`,
    };
}

/** What the hook is given. */
interface FreezeParams {
    model: BrowserViewModel;
    placeholderRef: () => HTMLDivElement | undefined;
    paneRect: () => PaneRect;
    paneCreated: () => boolean;
    diag: (msg: string) => void;
}

/** What the freeze-frame holds and is waiting on. */
interface FreezeState {
    params: FreezeParams;
    // Full data URL of the held frame (null = no frame).
    snapshot: Accessor<string | null>;
    setSnapshot: Setter<string | null>;
    // Inline geometry for the <img> — aligned to the native pane's
    // DPR-rounded physical rect, not the placeholder's fractional CSS box;
    // a ≤0.5px mismatch shifts content at the live→snapshot swap (visible
    // as a small jerk).
    setStyle: Setter<JSX.CSSProperties>;
    // Monotonic guard: bumped when a clear fires or a new capture starts, so
    // a stale in-flight capture can't resurrect an outdated snapshot.
    token: number;
    // The one in-flight capture, if any — a clip hit that lands while a
    // prewarm capture is still running must wait on IT, not start a second
    // capture from scratch (which would forfeit the prewarm head start).
    inflight: Promise<void> | null;
    clearTimer: ReturnType<typeof setTimeout> | null;
}

function cancelFreezeClear(f: FreezeState): void {
    if (f.clearTimer) {
        clearTimeout(f.clearTimer);
        f.clearTimer = null;
    }
}

function scheduleFreezeClear(f: FreezeState, ms: number): void {
    cancelFreezeClear(f);
    f.clearTimer = setTimeout(() => {
        f.clearTimer = null;
        f.token++;
        f.setSnapshot(null);
    }, ms);
}

function captureFreezeFrame(f: FreezeState): Promise<void> {
    const { model, placeholderRef, paneRect, diag } = f.params;
    const myToken = ++f.token;
    // JPEG q80, not PNG: encode is 3-5x faster in the renderer and the
    // payload much smaller. Capture latency directly gates how long
    // flushClip defers the airspace hide, which in turn is how long the
    // menu's over-pane portion stays covered by the live pane.
    const p = getApi()
        .browserPanes.screenshot(model.blockId, { format: "jpeg", quality: 80 })
        .then(
            (data) =>
                new Promise<void>((resolve) => {
                    if (myToken !== f.token || !data?.png_base64) return resolve();
                    const pr = paneRect();
                    const dpr = window.devicePixelRatio || 1;
                    const box = placeholderRef()!.getBoundingClientRect();
                    f.setStyle(freezeGeometry(box, pr, dpr));
                    f.setSnapshot(`data:image/jpeg;base64,${data.png_base64}`);
                    // Resolve one frame later so the <img> has actually
                    // painted before flushClip releases the hide IPC —
                    // that paint-before-hide ordering is the whole
                    // anti-flash mechanism.
                    requestAnimationFrame(() => resolve());
                }),
        )
        .catch((err) => diag(`[freeze] capture failed: ${err}`));
    f.inflight = p;
    void p.finally(() => {
        if (f.inflight === p) f.inflight = null;
    });
    return p;
}

// Which panes hide whole under a menu, and so need this: `hidesPaneUnderOverlay`.
function freezeGatesOpen(f: FreezeState): boolean {
    return hidesPaneUnderOverlay() && !!f.params.placeholderRef() && f.params.paneCreated() && !f.params.model.closed;
}

// Pre-warm: fired on pointer-down of menu anchors (see MenuTrigger and FlyoutMenu),
// ~100ms before the click completes and the menu opens. By the time the
// clip hit arrives the frame is usually already held (or at least in
// flight), so the hide releases within a frame or two and the menu
// paints in one piece instead of two.
function prewarmFreeze(f: FreezeState): void {
    if (!freezeGatesOpen(f)) return;
    cancelFreezeClear(f);
    if (!f.snapshot() && !f.inflight) void captureFreezeFrame(f);
    // Auto-drop if no overlay actually lands on this pane — a held
    // frame from an abandoned gesture must not survive to display stale
    // content minutes later.
    scheduleFreezeClear(f, 4000);
}

/** Whether any overlay rectangle (CSS px) lands on the pane's rectangle (back-converted to CSS px). */
function overlayHitsPane(pr: PaneRect, dpr: number, rects: { x: number; y: number; w: number; h: number }[]): boolean {
    const left = pr.x / dpr;
    const top = pr.y / dpr;
    const right = (pr.x + pr.width) / dpr;
    const bottom = (pr.y + pr.height) / dpr;
    return (
        pr.width > 0 &&
        pr.height > 0 &&
        rects.some((o) => o.x < right && o.x + o.w > left && o.y < bottom && o.y + o.h > top)
    );
}

function handleOverlayClip(f: FreezeState, e: Event): void {
    if (!freezeGatesOpen(f)) return;
    const detail = (e as CustomEvent).detail as
        | {
              rects?: { x: number; y: number; w: number; h: number }[];
              wait?: (p: Promise<unknown>) => void;
          }
        | undefined;
    const rects = detail?.rects ?? [];
    // Hit-test over the SAME rect the host's hide decision uses:
    // paneRect() (which insets floating-* windows by the edge-resize
    // border) back-converted to CSS px — NOT the raw placeholder box.
    // With the raw box, a menu overlapping only the floater's border
    // strip would freeze a pane the host never hides (reagent P1 on PR
    // #2098).
    const hit = overlayHitsPane(f.params.paneRect(), window.devicePixelRatio || 1, rects);
    if (!hit) {
        // Deferred clear: the reshow IPC needs a roundtrip, and once the
        // native pane is visible again it draws OVER the DOM anyway, so
        // a lingering snapshot is invisible — while dropping it
        // instantly would expose the bare placeholder for the reshow
        // gap. A reopen within the window cancels this and reuses the
        // held frame.
        scheduleFreezeClear(f, 250);
        return;
    }
    cancelFreezeClear(f);
    // Already frozen (held frame from a prewarm or a fast reopen) —
    // keep it; nothing to wait for.
    if (f.snapshot()) return;
    // Ask flushClip to hold the hide until the frame is painted (it
    // caps the wait, so a slow capture degrades to hide-then-snapshot
    // rather than blocking the menu). Reuse an in-flight prewarm
    // capture when one exists.
    detail?.wait?.(f.inflight ?? captureFreezeFrame(f));
}

export function useFreezeFrame(params: FreezeParams): FreezeFrame {
    const [freezeSnapshot, setFreezeSnapshot] = createSignal<string | null>(null);
    const [freezeStyle, setFreezeStyle] = createSignal<JSX.CSSProperties>({});
    const f: FreezeState = {
        params,
        snapshot: freezeSnapshot,
        setSnapshot: setFreezeSnapshot,
        setStyle: setFreezeStyle,
        token: 0,
        inflight: null,
        clearTimer: null,
    };
    const onFreezePrewarm = () => prewarmFreeze(f);
    const onOverlayClipChanged = (e: Event) => handleOverlayClip(f, e);

    onMount(() => {
        window.addEventListener("pane-overlay-clip-changed", onOverlayClipChanged);
        watchPaneClip();
        window.addEventListener("pane-freeze-prewarm", onFreezePrewarm);
        onCleanup(() => {
            window.removeEventListener("pane-overlay-clip-changed", onOverlayClipChanged);
            window.removeEventListener("pane-freeze-prewarm", onFreezePrewarm);
            cancelFreezeClear(f);
        });
    });

    return { freezeSnapshot, freezeStyle, onFreezePrewarm, onOverlayClipChanged };
}
