// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi } from "@/store/global";
import { Show, type JSX } from "solid-js";

const UpdateStatus = (): JSX.Element => {
    const updaterStatus = atoms.updaterStatusAtom;
    const updaterVersion = atoms.updaterVersionAtom;

    const icon = () => {
        switch (updaterStatus()) {
            case "available": return "●";
            case "downloading": return "↓";
            case "ready": return "↑";
            case "installing": return "⟳";
            case "error": return "✕";
            default: return null;
        }
    };

    const color = () => {
        switch (updaterStatus()) {
            case "available": return "var(--success-color)";
            case "downloading": return "var(--warning-color)";
            case "ready": return "var(--accent-color)";
            case "installing": return "var(--warning-color)";
            case "error": return "var(--error-color)";
            default: return null;
        }
    };

    const label = () => {
        switch (updaterStatus()) {
            case "available": {
                const ver = updaterVersion();
                return ver ? `Update ${ver} available` : "Update available";
            }
            case "downloading": return "Downloading update…";
            case "ready": return "Restart to update";
            case "installing": return "Installing…";
            case "error": return "Update failed";
            default: return null;
        }
    };

    const clickable = () => updaterStatus() === "ready" || updaterStatus() === "available";

    const handleClick = () => {
        if (updaterStatus() === "ready" || updaterStatus() === "available") {
            getApi().installAppUpdate();
        }
    };

    return (
        <Show when={updaterStatus() !== "up-to-date" && updaterStatus() !== "checking" && icon() !== null}>
            <div
                class={`status-bar-item${clickable() ? " clickable" : ""}`}
                onClick={clickable() ? handleClick : undefined}
                title={label()}
            >
                <span class="status-icon" style={{ color: color() }}>
                    {icon()}
                </span>
                <span style={{ color: color() }}>{label()}</span>
            </div>
        </Show>
    );
};

UpdateStatus.displayName = "UpdateStatus";

export { UpdateStatus };
