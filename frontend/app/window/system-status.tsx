// Copyright 2025, Command Line Inc.
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
import { WindowGrip } from "./window-grip";
import { WindowControlsRight } from "./window-controls.platform";
import { hostHas } from "@/util/host-caps";
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
            {/* A host with a window of its own (the desktop) has its grip and its caption buttons; a host without
                one (a browser tab) has neither: there is no window to drag. The grip sits between the widgets and
                the window controls (the AgentMux logo follows it on macOS). */}
            <Show when={hostHas("nativeWindowChrome")}>
                <WindowGrip />
                <WindowControlsRight />
            </Show>
        </div>
    );
};

export { SystemStatus, ConfigErrorMessage };
