// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Keeps the --dpr CSS custom property in sync with window.devicePixelRatio.
//
// Consumed by the pixel-snap primitives in mixins.scss (snap(), v-separator,
// etc.) so SCSS can express "1 device pixel" as `var(--snap)`. See
// docs/retros/RETRO_SUBPIXEL_RENDERING_RESEARCH_2026_04_26.md §4.1.

let mql: MediaQueryList | null = null;
let mqlListener: ((e: MediaQueryListEvent) => void) | null = null;

function applyDpr() {
    const dpr = window.devicePixelRatio || 1;
    document.documentElement.style.setProperty("--dpr", String(dpr));

    // matchMedia(`(resolution: <current>dppx)`) only fires when the
    // resolution moves AWAY from <current> — so re-arm on every change.
    if (mql && mqlListener) {
        mql.removeEventListener("change", mqlListener);
    }
    mql = window.matchMedia(`(resolution: ${dpr}dppx)`);
    mqlListener = () => applyDpr();
    mql.addEventListener("change", mqlListener);
}

export function setupDprTracking(): void {
    applyDpr();
}
