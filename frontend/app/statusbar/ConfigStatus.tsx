// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { modalsModel } from "@/app/store/modalmodel";
import { ConfigErrorMessage } from "@/app/window/system-status";
import { atoms } from "@/store/global";
import { useAtomValue } from "jotai";
import { memo } from "react";

const ConfigStatus = memo(() => {
    const fullConfig = useAtomValue(atoms.fullConfigAtom);

    if (fullConfig?.configerrors == null || fullConfig.configerrors.length === 0) {
        return null;
    }

    function handleClick() {
        modalsModel.pushModal("MessageModal", { children: <ConfigErrorMessage /> });
    }

    return (
        <div className="status-bar-item clickable" onClick={handleClick} title="Click to view config errors">
            <span className="status-icon" style={{ color: "var(--error-color)" }}>
                ⚠
            </span>
            <span style={{ color: "var(--error-color)" }}>Config error</span>
        </div>
    );
});

ConfigStatus.displayName = "ConfigStatus";

export { ConfigStatus };
