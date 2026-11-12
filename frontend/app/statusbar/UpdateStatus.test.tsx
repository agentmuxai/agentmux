// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The update chip and the Maintenance row, fed by srv's release check (store/app-update.ts). */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const updateStatus = vi.fn();
const updateCheck = vi.fn();
const updateDownload = vi.fn();
const upgradeRestart = vi.fn();
const upgradeCancel = vi.fn();
const storeCheck = vi.fn();
const settings = vi.fn();
const cameBack = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AppUpdateStatusCommand: (...a: unknown[]) => updateStatus(...a),
        AppUpdateCheckCommand: (...a: unknown[]) => updateCheck(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
// The version panel's clicks go through the host (approvals.appUpdate).
vi.mock("@/util/app-api", () => ({
    getApi: () => ({
        approvals: {
            appUpdate: (action: string, now?: boolean) =>
                action === "download" ? updateDownload() : action === "restart" ? upgradeRestart(now) : upgradeCancel(),
            storeUpdateCheck: () => storeCheck(),
            storeUpdateCameBack: () => cameBack(),
        },
    }),
}));
const openLink = vi.fn();
const installAppUpdate = vi.fn();
const storeUpdate = vi.fn();
const revealInFileExplorer = vi.fn();
vi.mock("@/store/global", async (importOriginal) => {
    const { createSignal } = await import("solid-js");
    const [status] = createSignal("up-to-date");
    const [version] = createSignal<string | null>(null);
    return {
        atoms: { updaterStatusAtom: status, updaterVersionAtom: version },
        getApi: () => ({ installAppUpdate, revealInFileExplorer, approvals: { storeUpdate }, listen: () => Promise.resolve(() => {}) }),
        openLink: (u: string) => openLink(u),
        getSettingsKeyAtom: (key: string) => () => settings(key),
    };
});

import {
    acceptAppUpdate,
    appUpdate,
    askStore,
    availableRelease,
    forgetStoreOffer,
    overtaken,
    storeAnswer,
    releaseDownload,
    setStoreStayed,
    storeStayedLine,
    stagedRelease,
    stagedUrgency,
} from "@/app/store/app-update";
import { notifications, setNotifications } from "@/app/store/flash-notifications";
import { MaintenanceSection } from "./MaintenanceSection";
import { RESTARTING_LINE } from "./RestartToUpdatePopover";
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
    updateCheck.mockReset();
    updateDownload.mockReset();
    upgradeRestart.mockReset();
    upgradeCancel.mockReset();
    openLink.mockReset();
    installAppUpdate.mockReset();
    storeUpdate.mockReset().mockResolvedValue({ outcome: "fallback", message: "" });
    storeCheck.mockReset().mockResolvedValue({ state: "available", message: "", version: "0.59.27" });
    settings.mockReset();
    cameBack.mockReset().mockResolvedValue({ state: "none", from: "", target: "" });
    setStoreStayed(null);
    setNotifications([]);
    forgetStoreOffer();
    revealInFileExplorer.mockReset().mockResolvedValue(undefined);
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

    it("keeps the newer of two statuses, whichever arrives first", () => {
        acceptAppUpdate({ ...AVAILABLE, rev: 5_000, notes: "newer" });
        acceptAppUpdate({ ...AVAILABLE, rev: 4_999, notes: "older" });
        expect(appUpdate()?.notes).toBe("newer");
        acceptAppUpdate({ ...AVAILABLE, rev: 5_001, notes: "newest" });
        expect(appUpdate()?.notes).toBe("newest");
        // A status that doesn't say is taken as it comes, and so is one from long before: another run of srv.
        expect(overtaken(0, 5_001)).toBe(false);
        expect(overtaken(5_000, 0)).toBe(false);
        expect(overtaken(5_000, 5_000)).toBe(false);
        expect(overtaken(5_000, 5_000 + 11 * 60 * 1000)).toBe(false);
        expect(overtaken(5_000, 5_000 + 9 * 60 * 1000)).toBe(true);
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

describe("UpdateStatus chip, once srv has staged the release by itself", () => {
    it("shows no chip while the release is still downloading: there is nothing to click", async () => {
        updateStatus.mockResolvedValue(download("downloading"));
        render(() => <UpdateStatus />);
        await Promise.resolve();
        await Promise.resolve();
        expect(screen.queryByText("Update to 0.59.27")).toBeNull();
        expect(screen.queryByText("Restart to update")).toBeNull();
        cleanup();
        updateStatus.mockResolvedValue(download("unpacking"));
        render(() => <UpdateStatus />);
        await Promise.resolve();
        await Promise.resolve();
        expect(screen.queryByText("Restart to update")).toBeNull();
    });

    it("reads Restart to update, and the click opens What's new / Later / Restart now without restarting", async () => {
        updateStatus.mockResolvedValue(download("ready", { folder: "C:/apps/x" }));
        render(() => <UpdateStatus />);
        fireEvent.click(await screen.findByText("Restart to update"));
        expect(await screen.findByText("AgentMux 0.59.27 is ready")).toBeTruthy();
        expect(upgradeRestart).not.toHaveBeenCalled();
        expect(screen.getByRole("button", { name: "What's new" })).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "What's new" }));
        expect(openLink).toHaveBeenCalledWith("https://github.com/agentmuxai/agentmux/releases/tag/v0.59.27");
        fireEvent.click(screen.getByRole("button", { name: "Later" }));
        expect(screen.queryByText("AgentMux 0.59.27 is ready")).toBeNull();
        // The chip stays: Later is not a dismissal of the update.
        expect(screen.getByText("Restart to update")).toBeTruthy();
    });

    it("Restart now asks for the same restart as the version panel, through the host", async () => {
        updateStatus.mockResolvedValue(download("ready", { folder: "C:/apps/x" }));
        upgradeRestart.mockResolvedValue(download("waiting", { folder: "C:/apps/x", waiting_for: 2 }));
        render(() => <UpdateStatus />);
        fireEvent.click(await screen.findByText("Restart to update"));
        fireEvent.click(await screen.findByRole("button", { name: "Restart now" }));
        expect(upgradeRestart).toHaveBeenCalledWith(false);
        // Agents are mid-turn: the panel says so and offers to stop waiting or go anyway.
        expect(await screen.findByText("Waiting for 2 agents to finish their turn.")).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Restart anyway" }));
        expect(upgradeRestart).toHaveBeenLastCalledWith(true);
        expect(installAppUpdate).not.toHaveBeenCalled();
    });

    /** The popover, open on a restart that is in `state`. */
    const openOn = async (state: string, extra: Record<string, unknown> = {}) => {
        updateStatus.mockResolvedValue(download(state, { folder: "C:/apps/x", ...extra }));
        render(() => <UpdateStatus />);
        fireEvent.click(await screen.findByText(state === "restarting" ? "Restarting…" : "Restart to update"));
        await screen.findByText("AgentMux 0.59.27 is ready");
    };
    const on = (...agents: string[]) => agents.map((agent) => ({ agent, why: "turn" }));

    it("a wait names the one agent it waits for, and what Restart anyway interrupts", async () => {
        await openOn("waiting", { waiting_for: 1, waiting_on: on("Vmer") });
        expect(screen.getByText("Waiting for Vmer to finish its turn.")).toBeTruthy();
        expect(screen.getByText("The restart starts by itself then. Restart anyway interrupts Vmer.")).toBeTruthy();
        expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
        expect(screen.getByRole("button", { name: "Restart anyway" })).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Restart now" })).toBeNull();
    });

    it("a wait for two agents names both", async () => {
        await openOn("waiting", { waiting_for: 2, waiting_on: on("Vmer", "Deke") });
        expect(screen.getByText("Waiting for Vmer and Deke to finish their turn.")).toBeTruthy();
        expect(screen.getByText("The restart starts by itself then. Restart anyway interrupts Vmer and Deke.")).toBeTruthy();
    });

    it("a wait for many agents counts them", async () => {
        await openOn("waiting", { waiting_for: 7, waiting_on: on("A", "B", "C", "D", "E", "F", "G") });
        expect(screen.getByText("Waiting for 7 agents to finish their turn.")).toBeTruthy();
        expect(screen.getByText("The restart starts by itself then. Restart anyway interrupts their turns.")).toBeTruthy();
    });

    it("an agent whose request is being retried is named with what holds it up", async () => {
        await openOn("waiting", { waiting_for: 2, waiting_on: [...on("Deke"), { agent: "Vmer", why: "network_retry" }] });
        expect(screen.getByText("Waiting for Deke to finish its turn.")).toBeTruthy();
        expect(screen.getByText("Waiting for Vmer, which is retrying after a network error.")).toBeTruthy();
        expect(screen.getByRole("button", { name: "Restart anyway" })).toBeTruthy();
    });

    it("never says it waits for 0 agents: a wait for nobody shows Later and Restart now, and is noted once", async () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        await openOn("waiting", { waiting_for: 0 });
        expect(screen.queryByText(/Waiting for/)).toBeNull();
        expect(screen.queryByRole("button", { name: "Restart anyway" })).toBeNull();
        expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
        expect(screen.getByRole("button", { name: "Later" })).toBeTruthy();
        expect(screen.getByRole("button", { name: "Restart now" })).toBeTruthy();
        // The same again changes nothing and says nothing more.
        acceptAppUpdate(download("waiting", { folder: "C:/apps/x", waiting_for: 0, received: 51 }));
        expect(screen.queryByText(/Waiting for/)).toBeNull();
        const said = warn.mock.calls.filter((c) => String(c[0]).includes("waiting, but for nobody"));
        expect(said).toHaveLength(1);
        expect(String(said[0][1])).toContain('"state":"waiting"');
        warn.mockRestore();
    });

    it("when the wait ends the panel says Restarting now, with nothing left to press but What's new", async () => {
        await openOn("waiting", { waiting_for: 1, waiting_on: on("Vmer") });
        acceptAppUpdate(download("restarting", { folder: "C:/apps/x" }));
        expect(await screen.findByText(RESTARTING_LINE)).toBeTruthy();
        expect(screen.queryByText(/Waiting for/)).toBeNull();
        for (const name of ["Cancel", "Restart anyway", "Restart now", "Later"]) expect(screen.queryByRole("button", { name })).toBeNull();
        expect(screen.getByRole("button", { name: "What's new" })).toBeTruthy();
        expect(screen.getByText("Restarting…")).toBeTruthy();
    });

    it("the chip's tip is not drawn over the open panel, and says what is happening once the restart is under way", async () => {
        // Closed: the tip says what the chip does, and during the restart what is happening.
        updateStatus.mockResolvedValue(download("ready", { folder: "C:/apps/x" }));
        render(() => <UpdateStatus />);
        const chip = await screen.findByLabelText("Update: Restart to update");
        expect(chip.getAttribute("data-tip")).toBe("AgentMux has downloaded an update: restart to install it");
        // Open: no tip, which would draw over the panel's buttons.
        fireEvent.click(chip);
        await screen.findByText("AgentMux 0.59.27 is ready");
        expect(chip.getAttribute("data-tip")).toBeNull();
        // Closed again during the restart: the old wording would be out of date.
        fireEvent.click(screen.getByRole("button", { name: "Later" }));
        acceptAppUpdate(download("restarting", { folder: "C:/apps/x" }));
        await screen.findByLabelText("Update: Restarting…");
        expect(screen.getByLabelText("Update: Restarting…").getAttribute("data-tip")).toBe("AgentMux is restarting to install the update");
    });

    it("the click's own answer, arriving after the event for what came next, does not put the panel back", async () => {
        // Nobody is in a turn: srv answers the click and asks the launcher a moment later. The event for
        // that reaches the window first, then the click's answer, which is the older of the two.
        updateStatus.mockResolvedValue({ ...download("ready", { folder: "C:/apps/x" }), rev: 1_000 });
        let answer: (status: unknown) => void = () => {};
        upgradeRestart.mockReturnValue(new Promise((resolve) => (answer = resolve)));
        render(() => <UpdateStatus />);
        fireEvent.click(await screen.findByText("Restart to update"));
        fireEvent.click(await screen.findByRole("button", { name: "Restart now" }));
        acceptAppUpdate({ ...download("restarting", { folder: "C:/apps/x" }), rev: 1_002 });
        expect(await screen.findByText(RESTARTING_LINE)).toBeTruthy();
        answer({ ...download("waiting", { folder: "C:/apps/x", waiting_for: 0 }), rev: 1_001 });
        await Promise.resolve();
        await Promise.resolve();
        expect(appUpdate()?.download?.state).toBe("restarting");
        expect(screen.getByText(RESTARTING_LINE)).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Restart anyway" })).toBeNull();
    });

    it("says why a restart that was refused did not start", async () => {
        updateStatus.mockResolvedValue(download("ready", { folder: "C:/apps/x" }));
        upgradeRestart.mockRejectedValue(new Error("an upgrade is already under way"));
        render(() => <UpdateStatus />);
        fireEvent.click(await screen.findByText("Restart to update"));
        fireEvent.click(await screen.findByRole("button", { name: "Restart now" }));
        expect(await screen.findByText("an upgrade is already under way")).toBeTruthy();
    });

    it("a release srv found but could not stage still opens the version panel", async () => {
        updateStatus.mockResolvedValue(download("failed", { error: "disk full" }));
        const opened = vi.fn();
        window.addEventListener("agentmux:open-version-panel", opened);
        render(() => <UpdateStatus />);
        fireEvent.click(await screen.findByText("Update to 0.59.27"));
        expect(opened).toHaveBeenCalledTimes(1);
        window.removeEventListener("agentmux:open-version-panel", opened);
    });
});

describe("staged release helpers", () => {
    it("stagedRelease is the release once it is ready, waiting or restarting, never before", () => {
        expect(stagedRelease(download("ready") as never)).toEqual({ version: "0.59.27", state: "ready", waitingFor: 0, waitingOn: [] });
        const vmer = [{ agent: "Vmer", why: "turn" }];
        expect(stagedRelease(download("waiting", { waiting_for: 1, waiting_on: vmer }) as never)?.waitingOn).toEqual(vmer);
        expect(stagedRelease(download("waiting", { waiting_for: 3 }) as never)?.waitingFor).toBe(3);
        expect(stagedRelease(download("restarting") as never)?.state).toBe("restarting");
        for (const state of ["downloading", "unpacking", "failed"]) expect(stagedRelease(download(state) as never)).toBeNull();
        expect(stagedRelease(PORTABLE as never)).toBeNull();
        // A download of an older release is not this release staged.
        expect(stagedRelease({ ...download("ready"), latest: "0.59.28" } as never)).toBeNull();
    });

    it("the chip warms after 2 days of the release being out and again after 4", () => {
        const published = "2026-11-01T00:00:00Z";
        const at = (days: number) => Date.parse(published) + days * 24 * 60 * 60 * 1000;
        const s = { ...download("ready"), published_at: published } as never;
        expect(stagedUrgency(s, at(1.9))).toBe("calm");
        expect(stagedUrgency(s, at(2))).toBe("warm");
        expect(stagedUrgency(s, at(3.9))).toBe("warm");
        expect(stagedUrgency(s, at(4))).toBe("hot");
        // No publish date: nothing nags.
        expect(stagedUrgency(download("ready") as never, at(30))).toBe("calm");
    });
});

describe("Maintenance update row", () => {
    it("shows the release, What's new opens its page on GitHub, and Download opens the release page", async () => {
        updateStatus.mockResolvedValue(AVAILABLE);
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("v0.59.27 is out")).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "What's new" }));
        expect(openLink).toHaveBeenCalledWith("https://github.com/agentmuxai/agentmux/releases/tag/v0.59.27");
        expect(screen.queryByText("- Sysinfo opens on a new Overview")).toBeNull();
        openLink.mockReset();
        fireEvent.click(screen.getByRole("button", { name: "Download" }));
        expect(openLink).toHaveBeenCalledWith(AVAILABLE.url);
        expect(installAppUpdate).not.toHaveBeenCalled();
    });

    it("What's new is offered when the release has no notes", async () => {
        updateStatus.mockResolvedValue({ ...AVAILABLE, notes: "" });
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("v0.59.27 is out")).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "What's new" }));
        expect(openLink).toHaveBeenCalledWith("https://github.com/agentmuxai/agentmux/releases/tag/v0.59.27");
    });

    it("a development build says which release is latest, and offers nothing to install", async () => {
        updateStatus.mockResolvedValue({ ...AVAILABLE, state: "local-build" });
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("Development build. The latest release is v0.59.27.")).toBeTruthy();
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

    it("says why when the folder can't be shown", async () => {
        updateStatus.mockResolvedValue(download("ready", { folder: "C:/apps/agentmux-0.59.27-x64-portable" }));
        revealInFileExplorer.mockRejectedValue(new Error("Failed to open in Explorer: not found"));
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Show folder" }));
        expect(await screen.findByText(/Couldn't open C:\/apps\/agentmux-0.59.27-x64-portable: Failed to open in Explorer: not found/)).toBeTruthy();
    });

    it("a staged release shows its folder; a failed one says why and can be retried", async () => {
        updateStatus.mockResolvedValue(download("ready", { folder: "C:/apps/agentmux-0.59.27-x64-portable" }));
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Show folder" }));
        expect(revealInFileExplorer).toHaveBeenCalledWith("C:/apps/agentmux-0.59.27-x64-portable");
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

    it("says what agents left running that the restart stops", async () => {
        const stopping = ["Korp: an AgentMux instance (`task dev TITLE=KorpForms`), running 9 h", "Naki-2: `node server.js`"];
        updateStatus.mockResolvedValue(download("waiting", { ...FOLDER, waiting_for: 1, stopping }));
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("The restart also stops what agents left running:")).toBeTruthy();
        for (const line of stopping) expect(screen.getByText(line)).toBeTruthy();
        cleanup();
        // Nothing left running: nothing said.
        updateStatus.mockResolvedValue(download("waiting", { ...FOLDER, waiting_for: 1, stopping: [] }));
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("Waiting for 1 agent to finish…")).toBeTruthy();
        expect(screen.queryByText("The restart also stops what agents left running:")).toBeNull();
    });

    it("an install older than the release upgrades from gets the floor version first", async () => {
        updateStatus.mockResolvedValue({ ...PORTABLE, can_download: false, install_first: "0.58.0" });
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Get v0.58.0" }));
        expect(openLink).toHaveBeenCalledWith("https://github.com/agentmuxai/agentmux/releases/tag/v0.58.0");
        expect(updateDownload).not.toHaveBeenCalled();
        expect(screen.getByText(/install v0.58.0 first, then upgrade/)).toBeTruthy();
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
        fireEvent.click(screen.getByRole("button", { name: "Upgrade & restart" }));
        expect(upgradeRestart).toHaveBeenCalledWith(false);
    });
});

describe("Maintenance: Check for update", () => {
    it("says this release is up to date, checks on demand and shows what it found", async () => {
        updateStatus.mockResolvedValue({ state: "up-to-date", current: "0.59.27", latest: "0.59.27", checked_ms: 1, install: "windows-portable", can_download: false });
        updateCheck.mockResolvedValue({ ...PORTABLE, current: "0.59.27", latest: "0.59.28" });
        render(() => <MaintenanceSection />);
        expect(await screen.findByText(/^v0\.59\.27 is up to date \(checked /)).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Check for update" }));
        expect(updateCheck).toHaveBeenCalledTimes(1);
        expect(await screen.findByText("v0.59.28 is out")).toBeTruthy();
        // A release on offer: the button stays, to look again.
        expect(screen.queryByRole("button", { name: "Check for update" })).toBeNull();
        fireEvent.click(await screen.findByRole("button", { name: "Check again" }));
        expect(updateCheck).toHaveBeenCalledTimes(2);
    });

    it("can't check while a download or a restart into the offered release is under way", async () => {
        for (const state of ["downloading", "unpacking", "waiting", "restarting"]) {
            updateStatus.mockResolvedValue(download(state));
            const { unmount } = render(() => <MaintenanceSection />);
            const button = (await screen.findByRole("button", { name: "Check again" })) as HTMLButtonElement;
            await waitFor(() => expect(button.disabled).toBe(true));
            unmount();
        }
        for (const state of ["ready", "failed"]) {
            updateStatus.mockResolvedValue(download(state));
            const { unmount } = render(() => <MaintenanceSection />);
            const button = (await screen.findByRole("button", { name: "Check again" })) as HTMLButtonElement;
            await waitFor(() => expect(button.disabled).toBe(false));
            unmount();
        }
    });

    it("says why a check failed", async () => {
        updateStatus.mockResolvedValue({ state: "unknown", current: "0.59.27", checked_ms: 0, install: "windows-portable", can_download: false });
        updateCheck.mockRejectedValue(new Error("couldn't reach GitHub: timed out"));
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("Not checked for updates yet")).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Check for update" }));
        expect(await screen.findByText(/couldn't reach GitHub: timed out/)).toBeTruthy();
    });

    it("a development build says which release is latest, and can check again", async () => {
        updateStatus.mockResolvedValue({ state: "local-build", current: "0.59.27", latest: "0.59.28", checked_ms: 1, install: "unknown", can_download: false });
        render(() => <MaintenanceSection />);
        expect(await screen.findByText(/Development build. The latest release is v0.59.28/)).toBeTruthy();
        expect(screen.getByRole("button", { name: "Check for update" })).toBeTruthy();
    });
});


describe("Maintenance on a Microsoft Store install", () => {
    const STORE = { ...AVAILABLE, install: "windows-store", can_download: false };
    const listed = (version: string) => ({ state: "available", message: "", version });
    const STORE_PAGE = "ms-windows-store://pdp/?productid=9P9QCXNNCRK3";
    const RELEASE_0_59_27 = "https://github.com/agentmuxai/agentmux/releases/tag/v0.59.27";

    it("offers the Store, never the release page or a download, once the Store lists the update", async () => {
        updateStatus.mockResolvedValue(STORE);
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Update in the Store" }));
        // The Store is asked to install (the default mock says it can't), then its page opens.
        await vi.waitFor(() => expect(openLink).toHaveBeenCalledTimes(1));
        expect(openLink).toHaveBeenCalledWith(STORE_PAGE);
        expect(updateDownload).not.toHaveBeenCalled();
        expect(installAppUpdate).not.toHaveBeenCalled();
        expect(screen.queryByRole("button", { name: "Download" })).toBeNull();
        expect(storeCheck).toHaveBeenCalledTimes(1);
    });

    it("offers the version the Store lists, not GitHub's latest tag, and links that version's notes", async () => {
        // GitHub's latest is newer than what the Store has: 0.59.29 was never submitted.
        updateStatus.mockResolvedValue({ ...STORE, latest: "0.59.29", url: "https://github.com/agentmuxai/agentmux/releases/tag/v0.59.29" });
        render(() => (
            <>
                <UpdateStatus />
                <MaintenanceSection />
            </>
        ));
        expect(await screen.findByText("v0.59.27 is in the Microsoft Store")).toBeTruthy();
        expect(screen.getByText("Update to 0.59.27")).toBeTruthy();
        expect(screen.queryByText(/0\.59\.29/)).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "What's new" }));
        expect(openLink).toHaveBeenCalledWith(RELEASE_0_59_27);
        fireEvent.click(screen.getByRole("button", { name: "Update in the Store" }));
        await vi.waitFor(() => expect(storeUpdate).toHaveBeenCalledWith("0.59.27"));
    });

    it("offers the update when GitHub knows of nothing newer, or was never asked", async () => {
        // srv's check never ran (no GitHub answer): the Store still has the say.
        updateStatus.mockResolvedValue({ state: "unknown", current: "0.59.26", checked_ms: 0, install: "windows-store", can_download: false });
        render(() => (
            <>
                <UpdateStatus />
                <MaintenanceSection />
            </>
        ));
        expect(await screen.findByRole("button", { name: "Update in the Store" })).toBeTruthy();
        expect(screen.getByText("Update to 0.59.27")).toBeTruthy();
        // The notes link is built from the Store's version: no GitHub answer needed.
        fireEvent.click(screen.getByRole("button", { name: "What's new" }));
        expect(openLink).toHaveBeenCalledWith(RELEASE_0_59_27);
    });

    it("when GitHub has a release the Store doesn't list: says it is published but not listed here, offers the Store page, never an install and no chip", async () => {
        storeCheck.mockResolvedValue({ state: "none", message: "" });
        updateStatus.mockResolvedValue(STORE);
        render(() => (
            <>
                <UpdateStatus />
                <MaintenanceSection />
            </>
        ));
        expect(
            await screen.findByText(/^v0\.59\.27 is published, but the Microsoft Store hasn't listed it for this PC yet \(checked .*\)\. This can take a while after the Store certifies a release\.$/)
        ).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Update in the Store" })).toBeNull();
        expect(screen.queryByRole("button", { name: "What's new" })).toBeNull();
        expect(screen.queryByText("Update to 0.59.27")).toBeNull();
        expect(screen.getByRole("button", { name: "Check for update" })).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Open Store page" }));
        expect(openLink).toHaveBeenCalledWith("ms-windows-store://pdp/?productid=9P9QCXNNCRK3");
        expect(storeUpdate).not.toHaveBeenCalled();
    });

    it("when GitHub has nothing newer either: the plain 'no newer version', with no Store page button", async () => {
        storeCheck.mockResolvedValue({ state: "none", message: "" });
        updateStatus.mockResolvedValue({ ...STORE, state: "up-to-date", latest: "0.59.26" });
        render(() => <MaintenanceSection />);
        expect(await screen.findByText(/^The Microsoft Store has no newer version \(checked /)).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Open Store page" })).toBeNull();
    });

    it("an update the Store lists with this build's own version, or an older one, is still an update: offered without a version", async () => {
        // The listed item's package is "the package that has an update available": Windows can give the installed one.
        for (const version of ["0.59.26", "0.59.25"]) {
            forgetStoreOffer();
            storeCheck.mockResolvedValue(listed(version));
            updateStatus.mockResolvedValue(STORE);
            render(() => <MaintenanceSection />);
            expect(await screen.findByRole("button", { name: "Update in the Store" })).toBeTruthy();
            expect(screen.getByText("Update available")).toBeTruthy();
            expect(screen.getByText("The Store closes AgentMux while it installs the update, then opens it again.")).toBeTruthy();
            // Never named after the installed version, and never called "not listed".
            expect(screen.queryByText(`v${version} is in the Microsoft Store`)).toBeNull();
            expect(screen.queryByText(/hasn't listed it/)).toBeNull();
            expect(screen.queryByText(/no newer version/)).toBeNull();
            expect(availableRelease()).toEqual({ version: "", url: "https://github.com/agentmuxai/agentmux/releases" });
            cleanup();
        }
    });

    it("installing an update the Store did not name passes no version, and a stay says so without one", async () => {
        storeCheck.mockResolvedValue(listed("0.59.26"));
        updateStatus.mockResolvedValue(STORE);
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Update in the Store" }));
        await vi.waitFor(() => expect(storeUpdate).toHaveBeenCalledWith(""));
        expect(storeStayedLine({ state: "stayed", from: "0.59.26", target: "" })).toBe(
            "The Store closed AgentMux to install an update, but this is still v0.59.26. Try Update in the Store again, or install it from the Microsoft Store app.",
        );
    });

    it("compares versions as numbers: 0.59.100 is newer than 0.59.27", async () => {
        acceptAppUpdate({ ...STORE, current: "0.59.27" });
        storeCheck.mockResolvedValue(listed("0.59.100"));
        render(() => <UpdateStatus />);
        await askStore();
        expect(await screen.findByText("Update to 0.59.100")).toBeTruthy();
        expect(availableRelease()?.url).toBe("https://github.com/agentmuxai/agentmux/releases/tag/v0.59.100");
    });

    it("an update the Store lists without a readable version is still offered, without a version to name", async () => {
        storeCheck.mockResolvedValue(listed(""));
        updateStatus.mockResolvedValue(STORE);
        render(() => (
            <>
                <UpdateStatus />
                <MaintenanceSection />
            </>
        ));
        expect(await screen.findByRole("button", { name: "Update in the Store" })).toBeTruthy();
        // The chip and the panel's row both say so, with no version.
        expect(screen.getAllByText("Update available")).toHaveLength(2);
        expect(screen.queryByText(/Update to/)).toBeNull();
    });

    it("when the Store can't be asked: says that and why, and doesn't guess from GitHub", async () => {
        storeCheck.mockResolvedValue({ state: "unknown", message: "no Store context: 0x80070490" });
        updateStatus.mockResolvedValue(STORE);
        render(() => (
            <>
                <UpdateStatus />
                <MaintenanceSection />
            </>
        ));
        expect(await screen.findByText("Couldn't ask the Microsoft Store (no Store context: 0x80070490).")).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Update in the Store" })).toBeNull();
        expect(screen.queryByText("Update to 0.59.27")).toBeNull();
    });

    it("when the Store never answers: says so plainly and offers nothing", async () => {
        storeCheck.mockResolvedValue({ state: "no_answer", message: "" });
        updateStatus.mockResolvedValue(STORE);
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("The Microsoft Store didn't answer; try again later.")).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Update in the Store" })).toBeNull();
        expect(screen.queryByText("Update to 0.59.27")).toBeNull();
    });

    it("a host whose Store call throws is the same: unknown, nothing offered", async () => {
        storeCheck.mockRejectedValue(new Error("host went away"));
        updateStatus.mockResolvedValue(STORE);
        render(() => <MaintenanceSection />);
        expect(await screen.findByText("Couldn't ask the Microsoft Store (host went away).")).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Update in the Store" })).toBeNull();
    });

    it("Check for update asks the Store, not GitHub, and the button appears when it lists the update", async () => {
        storeCheck.mockResolvedValue({ state: "none", message: "" });
        updateStatus.mockResolvedValue(STORE);
        render(() => <MaintenanceSection />);
        expect(await screen.findByText(/^v0\.59\.27 is published, but the Microsoft Store hasn't listed it for this PC yet/)).toBeTruthy();
        storeCheck.mockResolvedValue(listed("0.59.27"));
        fireEvent.click(screen.getByRole("button", { name: "Check for update" }));
        expect(await screen.findByRole("button", { name: "Update in the Store" })).toBeTruthy();
        expect(updateCheck).not.toHaveBeenCalled();
        expect(screen.queryByText(/no newer version/)).toBeNull();
        expect(storeCheck).toHaveBeenCalledTimes(2);
    });

    it("a status bar chip alone asks the Store a minute after the window starts, then every six hours", async () => {
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
        try {
            updateStatus.mockResolvedValue(STORE);
            render(() => <UpdateStatus />);
            await vi.advanceTimersByTimeAsync(59_000);
            expect(storeCheck).not.toHaveBeenCalled();
            expect(screen.queryByText("Update to 0.59.27")).toBeNull();
            await vi.advanceTimersByTimeAsync(1_000);
            expect(storeCheck).toHaveBeenCalledTimes(1);
            expect(screen.getByText("Update to 0.59.27")).toBeTruthy();
            // The later asks are quiet: the answer on show stays until the new one comes.
            storeCheck.mockResolvedValue({ state: "none", message: "" });
            // Six hours after the window started (a minute is already gone).
            await vi.advanceTimersByTimeAsync(6 * 60 * 60 * 1000 - 61_000);
            expect(storeCheck).toHaveBeenCalledTimes(1);
            await vi.advanceTimersByTimeAsync(1_000);
            expect(storeCheck).toHaveBeenCalledTimes(2);
            expect(storeAnswer()?.state).toBe("none");
            expect(screen.queryByText("Update to 0.59.27")).toBeNull();
        } finally {
            cleanup();
            vi.useRealTimers();
        }
    });

    it("with automatic update checks off, nothing asks the Store by itself; opening the panel still does", async () => {
        settings.mockImplementation((key: string) => (key === "update:auto" ? false : undefined));
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
        try {
            updateStatus.mockResolvedValue(STORE);
            render(() => <UpdateStatus />);
            await vi.advanceTimersByTimeAsync(7 * 60 * 60 * 1000);
            expect(storeCheck).not.toHaveBeenCalled();
            cleanup();
            // Opening the version panel is a look the person asked for.
            render(() => <MaintenanceSection />);
            await vi.advanceTimersByTimeAsync(0);
            expect(storeCheck).toHaveBeenCalledTimes(1);
        } finally {
            cleanup();
            vi.useRealTimers();
        }
    });

    it("does nothing with the Store on an install that isn't from it", async () => {
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
        try {
            updateStatus.mockResolvedValue(AVAILABLE);
            render(() => <UpdateStatus />);
            await vi.advanceTimersByTimeAsync(7 * 60 * 60 * 1000);
            expect(storeCheck).not.toHaveBeenCalled();
        } finally {
            cleanup();
            vi.useRealTimers();
        }
    });

    it("says before the click that the Store closes AgentMux and opens it again", async () => {
        updateStatus.mockResolvedValue(STORE);
        render(() => <MaintenanceSection />);
        await screen.findByRole("button", { name: "Update in the Store" });
        expect(screen.getByText("The Store closes AgentMux while it installs v0.59.27, then opens it again.")).toBeTruthy();
    });

    it("passes the release being installed to the host, so a restart can tell what it came back from", async () => {
        updateStatus.mockResolvedValue(STORE);
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Update in the Store" }));
        await vi.waitFor(() => expect(storeUpdate).toHaveBeenCalledWith("0.59.27"));
    });

    it("on a start that came back updated, tells the person once", async () => {
        cameBack.mockResolvedValue({ state: "updated", from: "0.59.26", target: "0.59.27" });
        render(() => <UpdateStatus />);
        await vi.waitFor(() => expect(notifications().map((n) => n.message)).toContain("Updated to v0.59.27 through the Microsoft Store."));
        expect(notifications()).toHaveLength(1);
    });

    it("on a start that stayed on the old version, says the install did not happen, in a toast and in the panel", async () => {
        cameBack.mockResolvedValue({ state: "stayed", from: "0.59.26", target: "0.59.27" });
        updateStatus.mockResolvedValue(STORE);
        render(() => (
            <>
                <UpdateStatus />
                <MaintenanceSection />
            </>
        ));
        const line = "The Store closed AgentMux to install v0.59.27, but this is still v0.59.26. Try Update in the Store again, or install it from the Microsoft Store app.";
        expect(await screen.findByText(line)).toBeTruthy();
        expect(notifications().some((n) => n.message === line && n.persistent)).toBe(true);
        // Trying again clears it.
        fireEvent.click(await screen.findByRole("button", { name: "Update in the Store" }));
        await vi.waitFor(() => expect(screen.queryByText(line)).toBeNull());
    });

    it("asks the Store to install it first, and opens the Store page only when the Store can't", async () => {
        updateStatus.mockResolvedValue(STORE);
        storeUpdate.mockResolvedValue({ outcome: "installed", message: "The Store installed the update. Open AgentMux again to use it." });
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Update in the Store" }));
        expect(await screen.findByText(/The Store installed the update/)).toBeTruthy();
        expect(storeUpdate).toHaveBeenCalledTimes(1);
        expect(openLink).not.toHaveBeenCalled();
    });

    it("tells that the Store has no newer version after all, withdrawing the offer, without opening the page", async () => {
        updateStatus.mockResolvedValue(STORE);
        storeUpdate.mockResolvedValue({ outcome: "none", message: "The Microsoft Store has no newer version." });
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Update in the Store" }));
        await vi.waitFor(() => expect(screen.queryByRole("button", { name: "Update in the Store" })).toBeNull());
        expect(screen.getAllByText(/The Microsoft Store has no newer version/).length).toBeGreaterThan(0);
        expect(openLink).not.toHaveBeenCalled();
    });

    it("falls back to the Store page, saying why, when the API refuses or throws", async () => {
        updateStatus.mockResolvedValue(STORE);
        storeUpdate.mockResolvedValue({ outcome: "fallback", message: "Couldn't ask the Store to install it (no Store context)." });
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Update in the Store" }));
        expect(await screen.findByText("Couldn't ask the Store to install it (no Store context). Opened the Microsoft Store page to finish the update.")).toBeTruthy();
        expect(openLink).toHaveBeenCalledWith(STORE_PAGE);
        openLink.mockReset();
        storeUpdate.mockRejectedValue(new Error("host went away"));
        fireEvent.click(screen.getByRole("button", { name: "Update in the Store" }));
        await vi.waitFor(() => expect(openLink).toHaveBeenCalledWith(STORE_PAGE));
        // With no reason to give, the line still says the page was opened.
        expect(await screen.findByText("Opened the Microsoft Store page to finish the update.")).toBeTruthy();
    });

    it("a Store install below the upgrade floor is not sent anywhere: the Store can't make the intermediate step", async () => {
        updateStatus.mockResolvedValue({ ...STORE, install_first: "0.58.0" });
        render(() => <MaintenanceSection />);
        expect(await screen.findByText(/The Microsoft Store can only install the latest version/)).toBeTruthy();
        expect(screen.getByText(/v0\.58\.0 or later/)).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Update in the Store" })).toBeNull();
        expect(screen.queryByRole("button", { name: /Get v0\.58\.0/ })).toBeNull();
        expect(screen.queryByRole("button", { name: "Download" })).toBeNull();
        expect(openLink).not.toHaveBeenCalled();
        // It says what to do instead, and links the page that explains it.
        expect(screen.getByText(/install v0\.58\.0 from GitHub first/)).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "How to move off the Store build" }));
        expect(openLink).toHaveBeenCalledWith("https://docs.agentmux.ai/installation/#moving-off-the-microsoft-store-build");
    });

    it("other installs keep their Download button and no Store note", async () => {
        updateStatus.mockResolvedValue(AVAILABLE);
        render(() => <MaintenanceSection />);
        expect(await screen.findByRole("button", { name: "Download" })).toBeTruthy();
        expect(screen.queryByText(/Microsoft Store/)).toBeNull();
        expect(storeCheck).not.toHaveBeenCalled();
    });

    it("other installs still take GitHub's check", async () => {
        updateStatus.mockResolvedValue({ ...AVAILABLE, state: "up-to-date" });
        updateCheck.mockResolvedValue(AVAILABLE);
        render(() => <MaintenanceSection />);
        fireEvent.click(await screen.findByRole("button", { name: "Check for update" }));
        expect(await screen.findByRole("button", { name: "Download" })).toBeTruthy();
        expect(updateCheck).toHaveBeenCalledTimes(1);
        expect(storeCheck).not.toHaveBeenCalled();
    });
});
