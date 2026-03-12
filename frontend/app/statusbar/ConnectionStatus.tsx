// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { modalsModel } from "@/app/store/modalmodel";
import { atoms } from "@/store/global";
import { For, Show, type JSX } from "solid-js";

const ConnectionStatusModal = ({ conns }: { conns: ConnStatus[] }): JSX.Element => (
    <div class="config-error-message">
        <h3>Active Connections</h3>
        <ul style={{ "list-style": "none", padding: "0", margin: "0" }}>
            <For each={conns}>
                {(c) => (
                    <li style={{ padding: "4px 0", display: "flex", gap: "8px", "align-items": "center" }}>
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
                        <span style={{ opacity: "0.5", "font-size": "0.9em" }}>{c.status}</span>
                    </li>
                )}
            </For>
        </ul>
    </div>
);

const ConnectionStatus = (): JSX.Element => {
    const allConnStatus = atoms.allConnStatus;

    const errorCount = () => allConnStatus().filter((c) => c.status === "error").length;
    const connectingCount = () => allConnStatus().filter((c) => c.status === "connecting" || c.status === "init").length;
    const total = () => allConnStatus().length;

    const icon = () => {
        if (errorCount() > 0) return "✕";
        if (connectingCount() > 0) return "◌";
        return "■";
    };

    const color = () => {
        if (errorCount() > 0) return "var(--error-color)";
        if (connectingCount() > 0) return "var(--warning-color)";
        return "var(--secondary-text-color)";
    };

    const label = () => {
        if (errorCount() > 0) return `${errorCount()} error`;
        if (connectingCount() > 0) return `${connectingCount()} connecting`;
        return `${total()} connection${total() !== 1 ? "s" : ""}`;
    };

    const handleClick = () => {
        modalsModel.pushModal("MessageModal", {
            children: <ConnectionStatusModal conns={allConnStatus()} />,
        });
    };

    return (
        <Show when={allConnStatus().length > 0}>
            <div class="status-bar-item clickable" title="Click to view connections" onClick={handleClick}>
                <span class="status-icon" style={{ color: color() }}>
                    {icon()}
                </span>
                <span style={{ color: color() }}>{label()}</span>
            </div>
        </Show>
    );
};

ConnectionStatus.displayName = "ConnectionStatus";

export { ConnectionStatus };
