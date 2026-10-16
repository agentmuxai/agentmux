// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The update chip and the Maintenance row, fed by srv's release check (store/app-update.ts). */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const updateStatus = vi.fn();
const updateDownload = vi.fn();
const upgradeRestart = vi.fn();
const upgradeCancel = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AppUpdateStatusCommand: (...a: unknown[]) => updateStatus(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
// The version panel's clicks go through the host (approvals.appUpdate).
vi.mock("@/app/store/app-api", () => ({
    getApi: () => ({
        approvals: {
            appUpdate: (action: string, now?: boolean) =>
                action === "download" ? updateDownload() : action === "restart" ? upgradeRestart(now) : upgradeCancel(),
        },
    }),
}));
const openLink = vi.fn();
const installAppUpdate = vi.fn();
const openNativePath = vi.fn();
const revealInFileExplorer = vi.fn();
vi.mock("@/store/global", async (importOriginal) => {
    const { createSignal } = await import("solid-js");
    const [status] = createSignal("up-to-date");
    const [version] = createSignal<string | null>(null);
    return {
        atoms: { updaterStatusAtom: status, updaterVersionAtom: version },
        getApi: () => ({ installAppUpdate, openNativePath, revealInFileExplorer, getBackendInfo: () => Promise.resolve({ pending_migrations: 0 }), listen: () => Promise.resolve(() => {}) }),
        openLink: (u: string) => openLink(u),
    };
});

import { acceptAppUpdate, appUpdate, availableRelease, releaseDownload } from "@/app/store/app-update";
import { MaintenanceSection } from "./MaintenanceSection";
import { UpdateStatus } from "./UpdateStatus";

const AVAILABLE = {
    state: "available",
    current: "0.59.26",
    latest: "0.59.27",
    url: "https://github.com/agentmuxai/agentmux/releases/tag/v0.59.27",
    notes: "- Sysinfo opens on a new Overview",
    checked_ms: 1,
    install: "windows-setup",
    can_download: false,
};

const PORTABLE = { ...AVAILABLE, install: "windows-portable", can_download: true };
const download = (state: string, extra: Record<string, unknown> = {}) => ({
    ...PORTABLE,
    download: { version: "0.59.27", state, received: 50, total: 200, ...extra },
});

beforeEach(() => {
    updateStatus.mockReset();
    updateDownload.mockReset();
    upgradeRestart.mockReset();
    upgradeCancel.mockReset();
    openLink.mockReset();
    installAppUpdate.mockReset();
    openNativePath.mockReset();
    acceptAppUpdate({ state: "unknown", current: "0.59.26", checked_ms: 0, install: "unknown", can_download: false });
});
afterEach(() => cleanup());

describe("app-update store", () => {
    it("takes srv's status and ignores anything malformed", () => {
        acceptAppUpdate(AVAILABLE);
        expect(availableRelease()).toEqual({ version: "0.59.27", url: AVAILABLE.url });
        acceptAppUpdate({ state: "nonsense", current: "x" });
        acceptAppUpdate(null);
        expect(appUpdate()?.state).toBe("available");
        acceptAppUpdate({ ...AVAILABLE, state: "up-to-date" });
        expect(availableRelease()).toBeNull();
        expect(availableRelease({ ...AVAILABLE, state: "local-build" })).toBeNull();
    });
});

describe("UpdateStatus chip", () => {
    it("is hidden while srv knows of nothing newer", async () => {
        updateStatus.mockResolvedValue({ state: "up-to-date", current: "0.59.27", latest: "0.59.27", checked_ms: 1 });
        render(() => <UpdateStatus />);
        await Promise.resolve();
        expect(screen.queryByText(/Update to/)).toBeNull();
    });

    it("names the release and opens the version panel, never installing", async () => {
        updateStatus.mockResolvedValue(AVAILABLE);
        const opened = vi.fn();
        window.addEventListener("agentmux:open-version-panel", opened);
        render(() => <UpdateStatus />);
        const chip = await screen.findByText("Update to 0.59.27");
        fireEvent.click(chip);
        expect(opened).toHaveBeenCalledTimes(1);
        expect(installAppUpdate).not.toHaveBeenCalled();
        window.removeEventListener("agentmux:open-version-panel", opened);
    });
});

describe("Maintenance update row", () => {
    it("shows the release, its notes, and Download opens the release page", async () => {
        updateStatus.mockResolvedValue(AVAILABLE);
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("v0.59.27 is out")).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "What's new" }));
        expect(screen.getByText("- Sysinfo opens on a new Overview")).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Download" }));
        expect(openLink).toHaveBeenCalledWith(AVAILABLE.url);
        expect(installAppUpdate).not.toHaveBeenCalled();
    });

    it("a local build says which release is latest, and offers nothing to install", async () => {
        updateStatus.mockResolvedValue({ ...AVAILABLE, state: "local-build" });
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("Local build. The latest release is v0.59.27.")).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Download" })).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "View" }));
        expect(openLink).toHaveBeenCalledWith(AVAILABLE.url);
    });
});

describe("Maintenance download, for an install AgentMux can upgrade itself", () => {
    it("Download asks srv to fetch it, and the row follows its progress", async () => {
        updateStatus.mockResolvedValue(PORTABLE);
        updateDownload.mockResolvedValue(download("downloading"));
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Download" }));
        expect(updateDownload).toHaveBeenCalledTimes(1);
        expect(openLink).not.toHaveBeenCalled();
        expect(await screen.findByText("Downloading v0.59.27… 25%")).toBeTruthy();
        acceptAppUpdate(download("unpacking"));
        expect(await screen.findByText("Unpacking v0.59.27…")).toBeTruthy();
    });

    it("a staged release shows its folder; a failed one says why and can be retried", async () => {
        updateStatus.mockResolvedValue(download("ready", { folder: "C:/apps/agentmux-0.59.27-x64-portable" }));
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Show folder" }));
        expect(openNativePath).toHaveBeenCalledWith("C:/apps/agentmux-0.59.27-x64-portable");
        expect(installAppUpdate).not.toHaveBeenCalled();

        acceptAppUpdate(download("failed", { error: "the download doesn't match the checksum GitHub lists for it" }));
        expect(await screen.findByText(/Download failed: the download doesn't match/)).toBeTruthy();
        updateDownload.mockResolvedValue(download("downloading"));
        fireEvent.click(screen.getByRole("button", { name: "Try again" }));
        expect(updateDownload).toHaveBeenCalledTimes(1);
        fireEvent.click(screen.getByRole("button", { name: "Release page" }));
        expect(openLink).toHaveBeenCalledWith(AVAILABLE.url);
    });

    it("a download of an older release is not shown against a newer one", () => {
        acceptAppUpdate({ ...download("ready"), latest: "0.59.28" });
        expect(releaseDownload()).toBeNull();
    });
});

describe("Maintenance: Upgrade & restart", () => {
    const FOLDER = { folder: "C:/apps/agentmux-0.59.27-x64-portable" };

    it("restarts once agents finish, and says how many it waits for", async () => {
        updateStatus.mockResolvedValue(download("ready", FOLDER));
        upgradeRestart.mockResolvedValue(download("waiting", { ...FOLDER, waiting_for: 2 }));
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Upgrade & restart" }));
        expect(upgradeRestart).toHaveBeenCalledWith(false);
        expect(await screen.findByText("Waiting for 2 agents to finish…")).toBeTruthy();

        upgradeRestart.mockResolvedValue(download("restarting", FOLDER));
        fireEvent.click(screen.getByRole("button", { name: "Restart now" }));
        expect(upgradeRestart).toHaveBeenLastCalledWith(true);
        expect(await screen.findByText("Restarting: answer the AgentMux dialog (No cancels)")).toBeTruthy();
        // Only the dialog can stop it now.
        expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
        expect(installAppUpdate).not.toHaveBeenCalled();
    });

    it("Cancel stops waiting, and a restart that didn't happen says why", async () => {
        updateStatus.mockResolvedValue(download("waiting", { ...FOLDER, waiting_for: 1 }));
        upgradeCancel.mockResolvedValue(download("ready", FOLDER));
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("Waiting for 1 agent to finish…")).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(upgradeCancel).toHaveBeenCalledTimes(1);
        expect(await screen.findByRole("button", { name: "Upgrade & restart" })).toBeTruthy();

        acceptAppUpdate(download("ready", { ...FOLDER, error: "AgentMux didn't restart: the upgrade was declined" }));
        expect(await screen.findByText(/the upgrade was declined/)).toBeTruthy();
    });

    it("a Cancel srv refuses says why", async () => {
        updateStatus.mockResolvedValue(download("waiting", { ...FOLDER, waiting_for: 1 }));
        upgradeCancel.mockRejectedValue(new Error("AgentMux is already restarting"));
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("Waiting for 1 agent to finish…")).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(await screen.findByText(/AgentMux is already restarting/)).toBeTruthy();
    });
});

describe("Maintenance on a setup.exe install", () => {
    it("a downloaded installer is shown in Explorer, never opened, and Upgrade & restart runs it", async () => {
        const SETUP = { ...AVAILABLE, install: "windows-setup", can_download: true };
        const installer = "C:/data/updates/0.59.27/AgentMux-0.59.27-x64-setup.exe";
        updateStatus.mockResolvedValue({
            ...SETUP,
            download: { version: "0.59.27", state: "ready", received: 1, total: 1, waiting_for: 0, installer },
        });
        upgradeRestart.mockResolvedValue({ ...SETUP, download: { version: "0.59.27", state: "restarting", received: 1, total: 1, waiting_for: 0, installer } });
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("v0.59.27 is ready")).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Show folder" })).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "Show installer" }));
        expect(revealInFileExplorer).toHaveBeenCalledWith(installer);
        expect(openNativePath).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Upgrade & restart" }));
        expect(upgradeRestart).toHaveBeenCalledWith(false);
    });
});
