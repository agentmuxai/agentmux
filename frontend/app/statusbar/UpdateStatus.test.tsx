// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The update chip and the Maintenance row, fed by srv's release check (store/app-update.ts). */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const updateStatus = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AppUpdateStatusCommand: (...a: unknown[]) => updateStatus(...a) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const openLink = vi.fn();
const installAppUpdate = vi.fn();
vi.mock("@/store/global", async (importOriginal) => {
    const { createSignal } = await import("solid-js");
    const [status] = createSignal("up-to-date");
    const [version] = createSignal<string | null>(null);
    return {
        atoms: { updaterStatusAtom: status, updaterVersionAtom: version },
        getApi: () => ({ installAppUpdate, getBackendInfo: () => Promise.resolve({ pending_migrations: 0 }), listen: () => Promise.resolve(() => {}) }),
        openLink: (u: string) => openLink(u),
    };
});

import { acceptAppUpdate, appUpdate, availableRelease } from "@/app/store/app-update";
import { MaintenanceSection } from "./MaintenanceSection";
import { UpdateStatus } from "./UpdateStatus";

const AVAILABLE = {
    state: "available",
    current: "0.59.26",
    latest: "0.59.27",
    url: "https://github.com/agentmuxai/agentmux/releases/tag/v0.59.27",
    notes: "- Sysinfo opens on a new Overview",
    checked_ms: 1,
};

beforeEach(() => {
    updateStatus.mockReset();
    openLink.mockReset();
    installAppUpdate.mockReset();
    acceptAppUpdate({ state: "unknown", current: "0.59.26", checked_ms: 0 });
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
