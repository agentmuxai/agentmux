// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SystemStatus - Right side of window header
 * Contains action widgets and window controls.
 * Update status and config errors have moved to StatusBar.
 */

import { atoms, getApi } from "@/store/global";
import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { For, Show, type JSX } from "solid-js";
import { ActionWidgets } from "./action-widgets";
import "./system-status.scss";


const ConfigErrorMessage = (): JSX.Element => {
    const fullConfig = atoms.fullConfigAtom;

    return (
        <Show
            when={fullConfig()?.configerrors != null && fullConfig().configerrors.length > 0}
            fallback={
                <div class="config-error-message">
                    <h3>Configuration Clean</h3>
                    <p>There are no longer any errors detected in your config.</p>
                </div>
            }
        >
            <Show
                when={fullConfig().configerrors.length === 1}
                fallback={
                    <div class="config-error-message">
                        <h3>Configuration Error</h3>
                        <ul>
                            <For each={fullConfig().configerrors}>
                                {(error) => (
                                    <li>
                                        {error.file}: {error.err}
                                    </li>
                                )}
                            </For>
                        </ul>
                    </div>
                }
            >
                <div class="config-error-message">
                    <h3>Configuration Error</h3>
                    <div>
                        {fullConfig().configerrors[0].file}: {fullConfig().configerrors[0].err}
                    </div>
                </div>
            </Show>
        </Show>
    );
};

// Windows 11 caption-button glyphs rendered as inline SVG so they are
// pixel-accurate regardless of which icon font happens to be installed.
// Stroke widths, proportions, and viewBox match what the Fluent design system
// uses for Segoe Fluent Icons U+E921 (minimize), U+E922 (maximize), U+E8BB
// (close). 10px glyph inside a 10px viewBox; the CSS gives the button its
// 46×32px hit area and the hover backgrounds.
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

const WindowActionButtons = (): JSX.Element => {
    const { dragProps } = useWindowDrag();
    const handleMinimize = () => {
        getApi().minimizeWindow();
    };

    const handleMaximize = () => {
        getApi().maximizeWindow();
    };

    const handleClose = () => {
        getApi().closeWindow();
    };

    return (
        <div class="window-action-buttons" {...dragProps}>
            <button
                class="window-action-btn minimize-btn"
                onClick={handleMinimize}
                title="Minimize"
                aria-label="Minimize"
                data-testid="window-minimize-btn"
                data-drag-region="false"
            >
                <MinimizeGlyph />
            </button>
            <button
                class="window-action-btn maximize-btn"
                onClick={handleMaximize}
                title="Maximize"
                aria-label="Maximize"
                data-testid="window-maximize-btn"
                data-drag-region="false"
            >
                <MaximizeGlyph />
            </button>
            <button
                class="window-action-btn close-btn"
                onClick={handleClose}
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

const SystemStatus = (): JSX.Element => {
    const { dragProps } = useWindowDrag();
    return (
        <div class="system-status" {...dragProps}>
            <ActionWidgets />
            <WindowActionButtons />
        </div>
    );
};

export { SystemStatus, ConfigErrorMessage };
