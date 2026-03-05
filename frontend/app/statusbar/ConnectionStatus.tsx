// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { modalsModel } from "@/app/store/modalmodel";
import { atoms } from "@/store/global";
import { useAtomValue } from "jotai";
import { memo } from "react";

const ConnectionStatusModal = ({ conns }: { conns: ConnStatus[] }) => (
    <div className="config-error-message">
        <h3>Active Connections</h3>
        <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {conns.map((c) => (
                <li key={c.connection} style={{ padding: "4px 0", display: "flex", gap: 8, alignItems: "center" }}>
                    <span
                        style={{
                            color:
                                c.status === "connected"
                                    ? "var(--accent-color)"
                                    : c.status === "error"
                                      ? "var(--error-color)"
                                      : "var(--warning-color)",
                        }}
                    >
                        {c.status === "connected" ? "●" : c.status === "error" ? "✕" : "◌"}
                    </span>
                    <span>{c.connection || "local"}</span>
                    <span style={{ opacity: 0.5, fontSize: "0.9em" }}>{c.status}</span>
                </li>
            ))}
        </ul>
    </div>
);

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

    const handleClick = () => {
        modalsModel.pushModal("MessageModal", {
            children: <ConnectionStatusModal conns={allConnStatus} />,
        });
    };

    return (
        <div className="status-bar-item clickable" title="Click to view connections" onClick={handleClick}>
            <span className="status-icon" style={{ color }}>
                {icon}
            </span>
            <span style={{ color }}>{label}</span>
        </div>
    );
});

ConnectionStatus.displayName = "ConnectionStatus";

export { ConnectionStatus };
