// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Singleton canvas for layout-free text measurement — synchronous,
// no DOM reflow, safe to call on every label change.
let _ctx: CanvasRenderingContext2D | null = null;

function getCtx(): CanvasRenderingContext2D | null {
    if (_ctx) return _ctx;
    try {
        const canvas = document.createElement("canvas");
        _ctx = canvas.getContext("2d");
    } catch {
        _ctx = null;
    }
    return _ctx;
}

// Non-text width budget:
//   10px left-pad + 10px right-pad + 4px gap + 16px close-btn + 12px slack = 52px
// Matches .tab-inner { padding: 0 10px } in tab.scss.
const TAB_PADDING_BUDGET = 52;
export const TAB_MIN_WIDTH = 60;
export const TAB_MAX_WIDTH = 260;

const DEFAULT_TAB_WIDTH = 160;

/**
 * Measures the natural pixel width for a workspace tab with the given label.
 * Uses a hidden canvas (no DOM reflow). Returns a clamped value ready to set
 * as `--tab-natural-width` on the `.tab-drop-wrapper` element.
 */
export function measureTabWidth(label: string): number {
    const ctx = getCtx();
    if (!ctx) return DEFAULT_TAB_WIDTH;

    // `.font` shorthand is not serialized by Chromium — always returns "".
    // Read fontFamily explicitly and pair with the tab label's hardcoded
    // font-size/weight (11px 400, from tab.scss .name).
    const fontFamily = getComputedStyle(document.documentElement).fontFamily || "system-ui";
    ctx.font = `400 11px ${fontFamily}`;

    const textWidth = ctx.measureText(label).width;
    const natural = Math.ceil(textWidth) + TAB_PADDING_BUDGET;
    return Math.min(Math.max(natural, TAB_MIN_WIDTH), TAB_MAX_WIDTH);
}
