// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SystemStatus - Right side of window header
 * Contains action widgets and window controls.
 * Update status and config errors have moved to StatusBar.
 */

import { atoms } from "@/store/global";
import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { For, Show, type JSX } from "solid-js";
import { ActionWidgets } from "./action-widgets";
import { WindowControlsRight } from "@/app/window/window-controls.platform";
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

const SystemStatus = (): JSX.Element => {
    const { dragProps } = useWindowDrag();
    return (
        <div class="system-status" {...dragProps}>
            <ActionWidgets />
            <WindowControlsRight />
        </div>
    );
};

export { SystemStatus, ConfigErrorMessage };
