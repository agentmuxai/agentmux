// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Windows 11 caption-button window controls.
 *
 * WindowControlsLeft  — null on Windows (no left-side controls needed).
 * WindowControlsRight — Win11-style Minimize / Maximize / Close buttons,
 *                        rendered on the RIGHT of the title bar via SystemStatus.
 */

import { getApi } from "@/store/global";
import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { type JSX } from "solid-js";
import "./window-controls.win32.scss";

// Windows 11 caption-button glyphs rendered as inline SVG so they are
// pixel-accurate regardless of which icon font happens to be installed.
// Stroke widths, proportions, and viewBox match what the Fluent design system
// uses for Segoe Fluent Icons U+E921 (minimize), U+E922 (maximize), U+E8BB
// (close). 10 px glyph inside a 10 px viewBox; the CSS gives the button its
// 46×32 px hit area and the hover backgrounds.
const MinimizeGlyph = (): JSX.Element => (
    <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
        <line x1="0" y1="5" x2="10" y2="5" stroke="currentColor" stroke-width="1" />
    </svg>
);

const MaximizeGlyph = (): JSX.Element => (
    <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
        <rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" stroke-width="1" />
    </svg>
);

const CloseGlyph = (): JSX.Element => (
    <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
        <line x1="0" y1="0" x2="10" y2="10" stroke="currentColor" stroke-width="1" />
        <line x1="10" y1="0" x2="0" y2="10" stroke="currentColor" stroke-width="1" />
    </svg>
);

const Win11CaptionButtons = (): JSX.Element => {
    const { dragProps } = useWindowDrag();

    return (
        <div class="window-action-buttons" {...dragProps}>
            <button
                class="window-action-btn minimize-btn"
                onClick={() => getApi().minimizeWindow()}
                title="Minimize"
                aria-label="Minimize"
                data-testid="window-minimize-btn"
                data-drag-region="false"
            >
                <MinimizeGlyph />
            </button>
            <button
                class="window-action-btn maximize-btn"
                onClick={() => getApi().maximizeWindow()}
                title="Maximize"
                aria-label="Maximize"
                data-testid="window-maximize-btn"
                data-drag-region="false"
            >
                <MaximizeGlyph />
            </button>
            <button
                class="window-action-btn close-btn"
                onClick={() => getApi().closeWindow().catch(console.error)}
                title="Close"
                aria-label="Close"
                data-testid="window-close-btn"
                data-drag-region="false"
            >
                <CloseGlyph />
            </button>
        </div>
    );
};

// On Windows there are no left-side controls.
export const WindowControlsLeft = (): JSX.Element => null;
export const WindowControlsRight = (): JSX.Element => <Win11CaptionButtons />;
