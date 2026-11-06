// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import {
    availableRelease,
    loadAppUpdate,
    loadStoreCameBack,
    openVersionPanel,
    releaseDownload,
    stagedRelease,
    stagedUrgency,
    updateOffered,
    useStoreOffer,
} from "@/store/app-update";
import { atoms, getApi } from "@/store/global";
import { createSignal, onMount, Show, type JSX } from "solid-js";
import { RestartToUpdatePopover } from "./RestartToUpdatePopover";

type ChipStatus = string | null | undefined;

const iconFor = (status: ChipStatus): string | null => {
    switch (status) {
        case "available": return "●";
        case "downloading": return "↓";
        case "ready": return "↑";
        case "installing": return "⟳";
        case "error": return "✕";
        default: return null;
    }
};

// A staged release is quiet at first and warms once the release has been out for days (Chrome's relaunch icon does the same).
const colorFor = (status: ChipStatus, urgency: "calm" | "warm" | "hot"): string | null => {
    switch (status) {
        case "available": return "var(--success-text-color)";
        case "downloading": return "var(--warning-text-color)";
        case "ready":
            return urgency === "hot" ? "var(--error-text-color)" : urgency === "warm" ? "var(--warning-text-color)" : "var(--accent-color)";
        case "installing": return "var(--warning-text-color)";
        case "error": return "var(--error-text-color)";
        default: return null;
    }
};

const labelFor = (status: ChipStatus, version: string | null | undefined, restarting: boolean): string | null => {
    switch (status) {
        case "available": return version ? `Update to ${version}` : "Update available";
        case "downloading": return "Downloading update…";
        case "ready": return restarting ? "Restarting…" : "Restart to update";
        case "installing": return "Installing…";
        case "error": return "Update failed";
        default: return null;
    }
};

const tipFor = (staged: boolean, fromSrv: boolean, clickable: boolean): string => {
    if (staged) return "AgentMux has downloaded an update: restart to install it";
    if (fromSrv) return "A newer AgentMux is out: see what's new";
    return clickable ? "Update available, click to install" : "Update status";
};

// The status-bar chip for a newer AgentMux. srv's release check
// (store/app-update.ts) says when one exists and downloads and stages it by
// itself (update/auto.rs): once staged the chip reads "Restart to update" and
// opens a small panel (What's new, Restart now, Later); while it downloads
// there is nothing to click, so no chip. The host updater's states
// (downloading, ready, …) are kept for a host that has one.
const UpdateStatus = (): JSX.Element => {
    onMount(() => {
        void loadAppUpdate();
        void loadStoreCameBack();
    });
    useStoreOffer();
    // A Store install shows the chip only once the Store itself lists the update.
    const fromSrv = () => (updateOffered() ? availableRelease() : null);
    const staged = () => stagedRelease();
    // srv is fetching it: nothing to click yet, and the version panel has the progress.
    const fetching = () => {
        const state = fromSrv() ? releaseDownload()?.state : undefined;
        return state === "downloading" || state === "unpacking";
    };
    // The panel is for a staged release and for the wait on agents after Restart now; once the
    // launcher has been asked, only its own dialog can stop it.
    const hasPanel = () => {
        const state = staged()?.state;
        return state === "ready" || state === "waiting";
    };
    const [panelOpen, setPanelOpen] = createSignal(false);
    let chipRef: HTMLDivElement | undefined;
    const updaterStatus = () => (staged() ? "ready" : fromSrv() ? "available" : atoms.updaterStatusAtom());
    const updaterVersion = () => fromSrv()?.version ?? atoms.updaterVersionAtom();
    const clickable = () => updaterStatus() === "ready" || updaterStatus() === "available";

    const handleClick = () => {
        // A release srv staged: What's new / Restart now / Later. A restart under way, or a
        // release srv found but did not stage: the version panel, which has the rest. The click
        // itself installs nothing: Restart now still ends in the launcher's own dialog.
        if (hasPanel()) {
            setPanelOpen(!panelOpen());
            return;
        }
        if (fromSrv()) {
            openVersionPanel();
            return;
        }
        if (clickable()) {
            getApi().installAppUpdate();
        }
    };

    const color = () => colorFor(updaterStatus(), stagedUrgency());
    const label = () => labelFor(updaterStatus(), updaterVersion(), staged()?.state === "restarting");

    return (
        <Show when={!fetching() && updaterStatus() !== "up-to-date" && updaterStatus() !== "checking" && iconFor(updaterStatus()) !== null}>
            <div
                ref={chipRef}
                class={`status-bar-item${clickable() ? " clickable" : ""}`}
                onClick={clickable() ? handleClick : undefined}
                data-tip={tipFor(!!staged(), !!fromSrv(), clickable())}
                aria-label={`Update: ${label()}`}
                aria-haspopup={hasPanel() ? "dialog" : undefined}
                aria-expanded={hasPanel() ? panelOpen() : undefined}
            >
                <span class="status-icon" style={{ color: color() ?? undefined }}>
                    {iconFor(updaterStatus())}
                </span>
                <span style={{ color: color() ?? undefined }}>{label()}</span>
            </div>
            <Show when={panelOpen() && hasPanel()}>
                <RestartToUpdatePopover anchor={chipRef} onClose={() => setPanelOpen(false)} />
            </Show>
        </Show>
    );
};

UpdateStatus.displayName = "UpdateStatus";

export { UpdateStatus };
