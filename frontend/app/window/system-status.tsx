// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * SystemStatus - Right side of window header
 * Contains action widgets and window controls.
 * Update status and config errors have moved to StatusBar.
 */

import { atoms, getApi } from "@/store/global";
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

const WindowActionButtons = (): JSX.Element => {
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
        <div class="window-action-buttons" data-tauri-drag-region="false">
            <button
                class="window-action-btn minimize-btn"
                onClick={handleMinimize}
                title="Minimize Window"
                data-testid="window-minimize-btn"
            >
                <i class="fa fa-window-minimize" />
            </button>
            <button
                class="window-action-btn maximize-btn"
                onClick={handleMaximize}
                title="Maximize Window"
                data-testid="window-maximize-btn"
            >
                <i class="fa fa-window-maximize" />
            </button>
            <button
                class="window-action-btn close-btn"
                onClick={handleClose}
                title="Close Window"
                data-testid="window-close-btn"
            >
                <i class="fa fa-times" />
            </button>
        </div>
    );
};

const SystemStatus = (): JSX.Element => {
    return (
        <div class="system-status">
            <ActionWidgets />
            <WindowActionButtons />
        </div>
    );
};

export { SystemStatus, ConfigErrorMessage };
