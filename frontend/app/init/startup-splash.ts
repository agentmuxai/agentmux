// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Startup splash control — the pulsing brain in index.html (`#startup-loading`)
// is a full-cover overlay (position:fixed; inset:0; solid bg; z-index 99999).
//
// It must stay up — covering the entire bootstrap + mount cascade — until the
// content-reveal gate (`tab-reveal.ts`) decides the window has settled, then
// cross-fade out. The previous behaviour removed it mid-mount (inside
// `initMux`), which exposed the bare chrome → empty-pane → piecemeal-mount
// flashes behind it. This is especially visible on tear-off (a pool window is
// shown instantly with the brain, then the brain was torn down before the
// torn-off content had rendered). Now the brain is removed only at the gate's
// "settled" moment, so the transition reads as brain → content with nothing
// uncovered in between.

/** Fade duration — keep in sync with `#startup-loading.fading` in index.html. */
const FADE_MS = 200;

/**
 * A promoted pool window (tear-off, new window) has its content ready when
 * the gate lifts; the long fade is for a cold start. Short cross-fade only.
 * SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md phase 1.
 */
const PROMOTED_FADE_MS = 90;

let promotedAt: number | null = null;
let onPromotedReveal: (() => void) | undefined;

/**
 * Called when the host promotes this pool window. `onReveal` runs once, when
 * the content reveals (the pane pool uses it to release its deferred refill).
 */
export function markPoolPromoted(onReveal?: () => void): void {
    promotedAt = performance.now();
    onPromotedReveal = onReveal;
}

/**
 * Cover the splash with a picture of the torn-off pane, so the floater shows
 * the pane from its first frame, and cross-fades to the live pane when the
 * gate lifts. SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md phase 3.1.
 */
export function showTearOffSnapshot(base64Jpeg: string): void {
    if (typeof document === "undefined") return;
    const el = document.getElementById("startup-loading");
    if (!el || el.dataset.amFading === "1") return;
    const img = document.createElement("img");
    img.alt = "";
    img.decoding = "sync";
    img.style.cssText = "position:absolute;inset:0;width:100%;height:100%;object-fit:fill;";
    img.src = `data:image/jpeg;base64,${base64Jpeg}`;
    el.appendChild(img);
    if (promotedAt != null) {
        console.log(`[tearoff-perf] snapshot shown ${Math.round(performance.now() - promotedAt)}ms after promote`);
    }
}

/**
 * Cross-fade and remove the startup splash. Idempotent and safe to call from
 * every reveal-gate lift: the first call fades it; once it's gone (the normal
 * case after the first window settles, and on every subsequent tab switch)
 * later calls are no-ops.
 */
export function fadeOutStartupSplash(): void {
    if (typeof document === "undefined") return;
    const el = document.getElementById("startup-loading");
    if (!el || el.dataset.amFading === "1") return;
    el.dataset.amFading = "1";
    let fadeMs = FADE_MS;
    if (promotedAt != null) {
        fadeMs = PROMOTED_FADE_MS;
        el.style.transitionDuration = `${fadeMs}ms`;
        console.log(`[tearoff-perf] reveal ${Math.round(performance.now() - promotedAt)}ms after promote`);
        onPromotedReveal?.();
        onPromotedReveal = undefined;
    }
    el.classList.add("fading");
    const done = () => el.remove();
    el.addEventListener("transitionend", done, { once: true });
    // Safety net in case `transitionend` never fires (reduced-motion forcing
    // an instant change, a display:none ancestor, etc.) so the splash can't be
    // left stuck on top of a ready window.
    setTimeout(done, fadeMs + 120);
}
