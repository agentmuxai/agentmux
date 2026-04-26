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

// Runtime parity check for v-separator consumers — see
// docs/retros/RETRO_SUBPIXEL_RENDERING_RESEARCH_2026_04_26 §1.5.
// SCSS can't enforce parity when the mixin is called with CSS
// custom properties (var(--tab-separator-width)), so we check the
// resolved values once at startup and warn in dev if a future
// theme change has violated the invariant.
function parityOf(px: string): number | null {
    const m = /^(-?\d+(?:\.\d+)?)px$/.exec(px.trim());
    if (!m) return null;
    const v = parseFloat(m[1]);
    return Number.isInteger(v) ? Math.abs(v) % 2 : null;
}

export function checkSeparatorParity(): void {
    const root = getComputedStyle(document.documentElement);
    const pairs: Array<[string, string]> = [
        ["--tab-separator-width", "--tab-separator-line"],
    ];
    for (const [slotVar, lineVar] of pairs) {
        const slot = root.getPropertyValue(slotVar);
        const line = root.getPropertyValue(lineVar);
        const sp = parityOf(slot);
        const lp = parityOf(line);
        if (sp == null || lp == null) continue;
        // Line of 0 px never paints, so parity doesn't matter.
        if (parseFloat(line) === 0) continue;
        if (sp !== lp) {
            console.warn(
                `[v-separator] parity mismatch: ${slotVar}=${slot.trim()} ` +
                `${lineVar}=${line.trim()} — line will not center on a device pixel.`
            );
        }
    }
}
