// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * macOS traffic-light window controls.
 *
 * WindowControlsLeft  — red/yellow/green circles rendered on the LEFT of the
 *                        title bar; show icon glyphs on hover.
 * WindowControlsRight — null on macOS (no right-side caption buttons needed).
 */

import { getApi } from "@/store/global";
import { type JSX } from "solid-js";
import "./window-controls.darwin.scss";

const TrafficLights = (): JSX.Element => {
    const handleClose = (e: MouseEvent) => {
        e.stopPropagation();
        getApi().closeWindow().catch(console.error);
    };
    const handleMinimize = (e: MouseEvent) => {
        e.stopPropagation();
        getApi().minimizeWindow();
    };
    const handleZoom = (e: MouseEvent) => {
        e.stopPropagation();
        getApi().maximizeWindow();
    };

    return (
        <div class="traffic-lights" data-testid="traffic-lights">
            {/* Close — red */}
            <button
                class="traffic-btn close-btn"
                onClick={handleClose}
                title="Close"
                aria-label="Close"
                data-testid="window-close-btn"
            >
                {/* × glyph */}
                <svg class="traffic-glyph" viewBox="0 0 10 10" width="6" height="6" aria-hidden="true">
                    <line x1="1.5" y1="1.5" x2="8.5" y2="8.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
                    <line x1="8.5" y1="1.5" x2="1.5" y2="8.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
                </svg>
            </button>

            {/* Minimize — yellow */}
            <button
                class="traffic-btn minimize-btn"
                onClick={handleMinimize}
                title="Minimize"
                aria-label="Minimize"
                data-testid="window-minimize-btn"
            >
                {/* – glyph */}
                <svg class="traffic-glyph" viewBox="0 0 10 10" width="6" height="6" aria-hidden="true">
                    <line x1="1.5" y1="5" x2="8.5" y2="5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
                </svg>
            </button>

            {/* Zoom / Full-screen — green */}
            <button
                class="traffic-btn zoom-btn"
                onClick={handleZoom}
                title="Zoom"
                aria-label="Zoom"
                data-testid="window-zoom-btn"
            >
                {/* ⤢ expand glyph (two inward-pointing arrows) */}
                <svg class="traffic-glyph" viewBox="0 0 10 10" width="6" height="6" aria-hidden="true">
                    <path
                        d="M1.5 1.5 L4.5 1.5 M1.5 1.5 L1.5 4.5 M8.5 8.5 L5.5 8.5 M8.5 8.5 L8.5 5.5"
                        stroke="currentColor"
                        stroke-width="1.5"
                        stroke-linecap="round"
                        fill="none"
                    />
                </svg>
            </button>
        </div>
    );
};

export const WindowControlsLeft = (): JSX.Element => <TrafficLights />;
// On macOS the right side of the header has no caption buttons.
export const WindowControlsRight = (): JSX.Element => null;
