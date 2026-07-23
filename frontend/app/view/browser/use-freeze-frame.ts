// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { invokeBrowserApi } from "@/app/platform/ipc";
import type { BrowserViewModel } from "./browser-model";
import type { PaneRect } from "./use-pane-rect-sync";

export interface FreezeFrame {
    freezeSnapshot: () => string | null;
    freezeStyle: () => JSX.CSSProperties;
    onFreezePrewarm: () => void;
    onOverlayClipChanged: (e: Event) => void;
}

// ── Freeze-frame while airspace-hidden (macOS/Linux only) ──────────
//
// When a DOM overlay (hamburger menu, modal, …) intersects this pane, the
// host hides the ENTIRE native pane surface — there is no SetWindowRgn-style
// hole punch off Windows — which exposes the bare placeholder and reads as
// the pane "going black". Instead, capture the pane's last rendered frame
// (CDP `Page.captureScreenshot` via the browser DOM API; the compositor
// keeps producing frames even while the overlay NSWindow is ordered out,
// verified empirically) and display it in the placeholder until the overlay
// clears. The frame is static — acceptable for menu-open-scale durations.
//
// Windows is excluded: its clip path punches a precise hole and never hides
// the pane, so there is nothing to freeze over.
export function useFreezeFrame(params: {
    model: BrowserViewModel;
    placeholderRef: () => HTMLDivElement | undefined;
    paneRect: () => PaneRect;
    paneCreated: () => boolean;
    diag: (msg: string) => void;
}): FreezeFrame {
    const { model, placeholderRef, paneRect, paneCreated, diag } = params;

    // Full data URL of the held frame (null = no frame).
    const [freezeSnapshot, setFreezeSnapshot] = createSignal<string | null>(null);
    // Inline geometry for the <img> — aligned to the native pane's
    // DPR-rounded physical rect, not the placeholder's fractional CSS box;
    // a ≤0.5px mismatch shifts content at the live→snapshot swap (visible
    // as a small jerk).
    const [freezeStyle, setFreezeStyle] = createSignal<JSX.CSSProperties>({});
    // Monotonic guard: bumped when a clear fires or a new capture starts, so
    // a stale in-flight capture can't resurrect an outdated snapshot.
    let freezeToken = 0;
    // The one in-flight capture, if any — a clip hit that lands while a
    // prewarm capture is still running must wait on IT, not start a second
    // capture from scratch (which would forfeit the prewarm head start).
    let freezeInflight: Promise<void> | null = null;
    let freezeClearTimer: ReturnType<typeof setTimeout> | null = null;

    const cancelFreezeClear = () => {
        if (freezeClearTimer) {
            clearTimeout(freezeClearTimer);
            freezeClearTimer = null;
        }
    };
    const scheduleFreezeClear = (ms: number) => {
        cancelFreezeClear();
        freezeClearTimer = setTimeout(() => {
            freezeClearTimer = null;
            freezeToken++;
            setFreezeSnapshot(null);
        }, ms);
    };
    const captureFreezeFrame = (): Promise<void> => {
        const myToken = ++freezeToken;
        // JPEG q80, not PNG: encode is 3-5x faster in the renderer and the
        // payload much smaller. Capture latency directly gates how long
        // flushClip defers the airspace hide, which in turn is how long the
        // menu's over-pane portion stays covered by the live pane.
        const p = invokeBrowserApi<{ png_base64: string }>("screenshot", {
            block_id: model.blockId,
            format: "jpeg",
            quality: 80,
        })
            .then(
                (data) =>
                    new Promise<void>((resolve) => {
                        if (myToken !== freezeToken || !data?.png_base64) return resolve();
                        const pr = paneRect();
                        const dpr = window.devicePixelRatio || 1;
                        const box = placeholderRef()!.getBoundingClientRect();
                        setFreezeStyle({
                            left: `${pr.x / dpr - box.x}px`,
                            top: `${pr.y / dpr - box.y}px`,
                            width: `${pr.width / dpr}px`,
                            height: `${pr.height / dpr}px`,
                        });
                        setFreezeSnapshot(`data:image/jpeg;base64,${data.png_base64}`);
                        // Resolve one frame later so the <img> has actually
                        // painted before flushClip releases the hide IPC —
                        // that paint-before-hide ordering is the whole
                        // anti-flash mechanism.
                        requestAnimationFrame(() => resolve());
                    }),
            )
            .catch((err) => diag(`[freeze] capture failed: ${err}`));
        freezeInflight = p;
        void p.finally(() => {
            if (freezeInflight === p) freezeInflight = null;
        });
        return p;
    };
    // Linux ONLY. Windows punches a SetWindowRgn hole (pane never hides —
    // nothing to freeze over), and macOS punches a CALayer hole mask
    // (ui_tasks/pane_hole_mask.rs) — the pane stays LIVE there, so freezing
    // is actively harmful there (forces a compositor frame, visible as a
    // jerk, and defers the clip IPC). Only Linux still whole-pane-hides and
    // needs the freeze-frame compensation.
    const freezeGatesOpen = (): boolean =>
        navigator.userAgent.includes("Linux") &&
        !navigator.userAgent.includes("Windows") &&
        !!placeholderRef() &&
        paneCreated() &&
        !model.closed;
    // Pre-warm: fired on pointer-down of menu anchors (see FlyoutMenu),
    // ~100ms before the click completes and the menu opens. By the time the
    // clip hit arrives the frame is usually already held (or at least in
    // flight), so the hide releases within a frame or two and the menu
    // paints in one piece instead of two.
    const onFreezePrewarm = () => {
        if (!freezeGatesOpen()) return;
        cancelFreezeClear();
        if (!freezeSnapshot() && !freezeInflight) void captureFreezeFrame();
        // Auto-drop if no overlay actually lands on this pane — a held
        // frame from an abandoned gesture must not survive to display stale
        // content minutes later.
        scheduleFreezeClear(4000);
    };
    const onOverlayClipChanged = (e: Event) => {
        if (!freezeGatesOpen()) return;
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
        const pr = paneRect();
        const dpr = window.devicePixelRatio || 1;
        const left = pr.x / dpr;
        const top = pr.y / dpr;
        const right = (pr.x + pr.width) / dpr;
        const bottom = (pr.y + pr.height) / dpr;
        const hit =
            pr.width > 0 &&
            pr.height > 0 &&
            rects.some((o) => o.x < right && o.x + o.w > left && o.y < bottom && o.y + o.h > top);
        if (!hit) {
            // Deferred clear: the reshow IPC needs a roundtrip, and once the
            // native pane is visible again it draws OVER the DOM anyway, so
            // a lingering snapshot is invisible — while dropping it
            // instantly would expose the bare placeholder for the reshow
            // gap. A reopen within the window cancels this and reuses the
            // held frame.
            scheduleFreezeClear(250);
            return;
        }
        cancelFreezeClear();
        // Already frozen (held frame from a prewarm or a fast reopen) —
        // keep it; nothing to wait for.
        if (freezeSnapshot()) return;
        // Ask flushClip to hold the hide until the frame is painted (it
        // caps the wait, so a slow capture degrades to hide-then-snapshot
        // rather than blocking the menu). Reuse an in-flight prewarm
        // capture when one exists.
        detail?.wait?.(freezeInflight ?? captureFreezeFrame());
    };

    onMount(() => {
        window.addEventListener("pane-overlay-clip-changed", onOverlayClipChanged);
        window.addEventListener("pane-freeze-prewarm", onFreezePrewarm);
        onCleanup(() => {
            window.removeEventListener("pane-overlay-clip-changed", onOverlayClipChanged);
            window.removeEventListener("pane-freeze-prewarm", onFreezePrewarm);
            cancelFreezeClear();
        });
    });

    return { freezeSnapshot, freezeStyle, onFreezePrewarm, onOverlayClipChanged };
}
