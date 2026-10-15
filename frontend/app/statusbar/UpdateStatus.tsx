// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { availableRelease, loadAppUpdate, openVersionPanel } from "@/store/app-update";
import { atoms, getApi } from "@/store/global";
import { onMount, Show, type JSX } from "solid-js";

// The status-bar chip for a newer AgentMux. srv's release check
// (store/app-update.ts) says when one exists; the host updater's states
// (downloading, ready, …) are kept for a host that has one.
const UpdateStatus = (): JSX.Element => {
    onMount(() => void loadAppUpdate());
    const fromSrv = () => availableRelease();
    const updaterStatus = () => (fromSrv() ? "available" : atoms.updaterStatusAtom());
    const updaterVersion = () => fromSrv()?.version ?? atoms.updaterVersionAtom();

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
                return ver ? `Update to ${ver}` : "Update available";
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
        // A release srv found: the version panel shows what's new and where
        // to get it. Nothing installs from a click on the chip.
        if (fromSrv()) {
            openVersionPanel();
            return;
        }
        if (updaterStatus() === "ready" || updaterStatus() === "available") {
            getApi().installAppUpdate();
        }
    };

    return (
        <Show when={updaterStatus() !== "up-to-date" && updaterStatus() !== "checking" && icon() !== null}>
            <div
                class={`status-bar-item${clickable() ? " clickable" : ""}`}
                onClick={clickable() ? handleClick : undefined}
                data-tip={fromSrv() ? "A newer AgentMux is out: see what's new" : clickable() ? "Update available, click to install" : "Update status"}
                aria-label={`Update: ${label()}`}
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
