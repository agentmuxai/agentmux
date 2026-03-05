// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi } from "@/store/global";
import { useAtomValue } from "jotai";
import { memo } from "react";

const UpdateStatus = memo(() => {
    const updaterStatus = useAtomValue(atoms.updaterStatusAtom);

    if (updaterStatus === "up-to-date" || updaterStatus === "checking") {
        return null;
    }

    let icon: string;
    let color: string;
    let label: string;
    let clickable = false;

    switch (updaterStatus) {
        case "downloading":
            icon = "↓";
            color = "var(--warning-color)";
            label = "Downloading update…";
            break;
        case "ready":
            icon = "↑";
            color = "var(--accent-color)";
            label = "Restart to update";
            clickable = true;
            break;
        case "installing":
            icon = "⟳";
            color = "var(--warning-color)";
            label = "Installing…";
            break;
        case "error":
            icon = "✕";
            color = "var(--error-color)";
            label = "Update failed";
            break;
        default:
            return null;
    }

    function handleClick() {
        if (updaterStatus === "ready") {
            getApi().installAppUpdate();
        }
    }

    return (
        <div
            className={`status-bar-item${clickable ? " clickable" : ""}`}
            onClick={clickable ? handleClick : undefined}
            title={label}
        >
            <span className="status-icon" style={{ color }}>
                {icon}
            </span>
            <span style={{ color }}>{label}</span>
        </div>
    );
});

UpdateStatus.displayName = "UpdateStatus";

export { UpdateStatus };
