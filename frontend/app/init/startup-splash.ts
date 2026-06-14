// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Startup splash control — the pulsing brain in index.html (`#startup-loading`)
// is a full-cover overlay (position:fixed; inset:0; solid bg; z-index 99999).
//
// It must stay up — covering the entire bootstrap + mount cascade — until the
// content-reveal gate (`tab-reveal.ts`) decides the window has settled, then
// cross-fade out. The previous behaviour removed it mid-mount (inside
// `initWave`), which exposed the bare chrome → empty-pane → piecemeal-mount
// flashes behind it. This is especially visible on tear-off (a pool window is
// shown instantly with the brain, then the brain was torn down before the
// torn-off content had rendered). Now the brain is removed only at the gate's
// "settled" moment, so the transition reads as brain → content with nothing
// uncovered in between.

/** Fade duration — keep in sync with `#startup-loading.fading` in index.html. */
const FADE_MS = 200;

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
    el.classList.add("fading");
    const done = () => el.remove();
    el.addEventListener("transitionend", done, { once: true });
    // Safety net in case `transitionend` never fires (reduced-motion forcing
    // an instant change, a display:none ancestor, etc.) so the splash can't be
    // left stuck on top of a ready window.
    setTimeout(done, FADE_MS + 120);
}
