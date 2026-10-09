// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Shown when the backend reports a different version than this UI's
// (app/store/srv-info.ts): one of them was updated under the other, and a
// reload brings the UI in line.

import { getApi } from "@/app/store/app-api";
import { UI_VERSION, versionSkew } from "@/app/store/srv-info";
import { Show, type JSX } from "solid-js";

const VersionSkewStatus = (): JSX.Element => (
    <Show when={versionSkew()}>
        {(srv) => (
            <div
                class="status-bar-item clickable"
                onClick={() => getApi().reloadWindow()}
                data-tip={`This window is version ${UI_VERSION}; the backend is ${srv()}. Click to reload.`}
                aria-label={`Backend is version ${srv()}: reload`}
            >
                <span class="status-icon" style={{ color: "var(--warning-color)" }}>
                    ⟳
                </span>
                <span style={{ color: "var(--warning-color)" }}>Reload for {srv()}</span>
            </div>
        )}
    </Show>
);

VersionSkewStatus.displayName = "VersionSkewStatus";

export { VersionSkewStatus };
