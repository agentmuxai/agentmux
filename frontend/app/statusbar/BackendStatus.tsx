// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms } from "@/store/global";
import { useAtomValue } from "jotai";
import { memo } from "react";

const BackendStatus = memo(() => {
    const backendStatus = useAtomValue(atoms.backendStatusAtom);

    let icon: string;
    let color: string;
    let label: string;
    let iconSpin = false;

    switch (backendStatus) {
        case "running":
            icon = "●";
            color = "var(--accent-color)";
            label = "Backend";
            break;
        case "connecting":
            icon = "◌";
            color = "var(--warning-color)";
            label = "Connecting…";
            iconSpin = true;
            break;
        case "crashed":
            icon = "●";
            color = "var(--error-color)";
            label = "Backend offline";
            break;
        default:
            return null;
    }

    return (
        <div className="status-bar-item" title={`Sidecar status: ${backendStatus}`}>
            <span className={`status-icon${iconSpin ? " status-icon-spin" : ""}`} style={{ color }}>
                {icon}
            </span>
            <span style={{ color }}>{label}</span>
        </div>
    );
});

BackendStatus.displayName = "BackendStatus";

export { BackendStatus };
