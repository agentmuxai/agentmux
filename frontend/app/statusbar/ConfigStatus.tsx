// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { modalsModel } from "@/app/store/modalmodel";
import { ConfigErrorMessage } from "@/app/window/system-status";
import { atoms } from "@/store/global";
import { Show, type JSX } from "solid-js";

const ConfigStatus = (): JSX.Element => {
    const fullConfig = atoms.fullConfigAtom;

    const handleClick = () => {
        modalsModel.pushModal("MessageModal", { children: <ConfigErrorMessage /> });
    };

    return (
        <Show when={fullConfig()?.configerrors != null && fullConfig().configerrors.length > 0}>
            <div class="status-bar-item clickable" onClick={handleClick} title="Click to view config errors">
                <span class="status-icon" style={{ color: "var(--error-color)" }}>
                    ⚠
                </span>
                <span style={{ color: "var(--error-color)" }}>Config error</span>
            </div>
        </Show>
    );
};

ConfigStatus.displayName = "ConfigStatus";

export { ConfigStatus };
