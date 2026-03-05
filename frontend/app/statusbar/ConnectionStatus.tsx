// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms } from "@/store/global";
import { useAtomValue } from "jotai";
import { memo } from "react";

const ConnectionStatus = memo(() => {
    const allConnStatus = useAtomValue(atoms.allConnStatus);

    if (allConnStatus.length === 0) {
        return null;
    }

    const errorCount = allConnStatus.filter((c) => c.status === "error").length;
    const connectingCount = allConnStatus.filter((c) => c.status === "connecting" || c.status === "init").length;
    const total = allConnStatus.length;

    let icon: string;
    let color: string;
    let label: string;

    if (errorCount > 0) {
        icon = "✕";
        color = "var(--error-color)";
        label = `${errorCount} error`;
    } else if (connectingCount > 0) {
        icon = "◌";
        color = "var(--warning-color)";
        label = `${connectingCount} connecting`;
    } else {
        icon = "■";
        color = "var(--secondary-text-color)";
        label = `${total} connection${total !== 1 ? "s" : ""}`;
    }

    return (
        <div className="status-bar-item" title="Active connections">
            <span className="status-icon" style={{ color }}>
                {icon}
            </span>
            <span style={{ color }}>{label}</span>
        </div>
    );
});

ConnectionStatus.displayName = "ConnectionStatus";

export { ConnectionStatus };
