// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Restoring the layout another build saved (srv `session_other_builds.rs`).

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OtherBuildLayout } from "@/types/rpc/OtherBuildLayout";

const h = vi.hoisted(() => ({
    OtherBuildLayoutsCommand: vi.fn(),
    RestoreOtherBuildLayoutCommand: vi.fn(),
    pushNotification: vi.fn(),
    reopenRestoredWindows: vi.fn(() => Promise.resolve()),
    actions: new Map<string, () => void>(),
}));

vi.mock("@/store/global", () => ({ pushNotification: h.pushNotification }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        OtherBuildLayoutsCommand: h.OtherBuildLayoutsCommand,
        RestoreOtherBuildLayoutCommand: h.RestoreOtherBuildLayoutCommand,
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { tag: "tab-client" } }));
vi.mock("@/app/store/window-identity", () => ({ windowId: () => "win-7" }));
vi.mock("@/app/init/restored-windows", () => ({ reopenRestoredWindows: h.reopenRestoredWindows }));
vi.mock("@/app/notification/usenotification", () => ({
    setNotificationAction: (key: string, fn: () => void) => h.actions.set(key, fn),
}));

import {
    chooseLayoutFromOtherBuild,
    layoutSummary,
    NOTICE_ID,
    offerLayoutFromOtherBuild,
    OFFER_MAX_AGE_MS,
    whenSaved,
} from "./restore-other-build";

const NOW = new Date(2026, 9, 20, 9, 0).getTime();

const layout = (over: Partial<OtherBuildLayout> = {}): OtherBuildLayout => ({
    channel: "local-main-1",
    version: "0.59.32",
    saved_at_ms: new Date(2026, 9, 20, 3, 28).getTime(),
    saved_at_exact: true,
    windows: 1,
    tabs: 4,
    panes: 34,
    agents: ["Korp", "AgentX", "Agent1"],
    ...over,
});

/** The notice pushed last. */
const notice = (): NotificationType => h.pushNotification.mock.calls.at(-1)?.[0];

/** Press the notice's button labelled `label`, as the toast does. */
function press(label: string): void {
    const a = notice().actions?.find((x) => x.label === label);
    expect(a, `a "${label}" button`).toBeDefined();
    h.actions.get(a!.actionKey)!();
}

beforeEach(() => {
    h.OtherBuildLayoutsCommand.mockReset();
    h.RestoreOtherBuildLayoutCommand.mockReset();
    h.pushNotification.mockReset();
    h.reopenRestoredWindows.mockClear();
    h.actions.clear();
});

describe("the offer at a cold start", () => {
    it("asks to restore the newest layout when this window shows the seeded default", async () => {
        h.OtherBuildLayoutsCommand.mockResolvedValue({ offer: true, layouts: [layout(), layout({ version: "0.59.31" })] });
        await offerLayoutFromOtherBuild(NOW);
        expect(h.OtherBuildLayoutsCommand).toHaveBeenCalledWith({ tag: "tab-client" }, { window_id: "win-7", offer_only: true });
        const n = notice();
        expect(n.id).toBe(NOTICE_ID);
        expect(n.persistent).toBe(true);
        expect(n.title).toBe("Restore the layout from 0.59.32 (closed 03:28 today): 4 tabs, 3 agents?");
        expect(n.actions?.map((a) => a.label)).toEqual(["Restore", "Not now"]);
    });

    it("says nothing when this build restored its own layout or the window is in use", async () => {
        h.OtherBuildLayoutsCommand.mockResolvedValue({ offer: false, layouts: [layout()] });
        await offerLayoutFromOtherBuild(NOW);
        expect(h.pushNotification).not.toHaveBeenCalled();
    });

    it("says nothing when no other build has a layout, or only an old one", async () => {
        h.OtherBuildLayoutsCommand.mockResolvedValue({ offer: true, layouts: [] });
        await offerLayoutFromOtherBuild(NOW);
        h.OtherBuildLayoutsCommand.mockResolvedValue({
            offer: true,
            layouts: [layout({ saved_at_ms: NOW - OFFER_MAX_AGE_MS - 1 })],
        });
        await offerLayoutFromOtherBuild(NOW);
        expect(h.pushNotification).not.toHaveBeenCalled();
    });

    it("Restore rebuilds that layout in this window", async () => {
        h.OtherBuildLayoutsCommand.mockResolvedValue({ offer: true, layouts: [layout()] });
        h.RestoreOtherBuildLayoutCommand.mockResolvedValue({
            workspace_id: "ws",
            tab_ids: ["t1", "t2", "t3", "t4"],
            replaced_default: true,
            other_windows: 0,
        });
        await offerLayoutFromOtherBuild(NOW);
        press("Restore");
        await vi.waitFor(() => expect(h.RestoreOtherBuildLayoutCommand).toHaveBeenCalledTimes(1));
        expect(h.RestoreOtherBuildLayoutCommand).toHaveBeenCalledWith(
            { tag: "tab-client" },
            { window_id: "win-7", channel: "local-main-1", version: "0.59.32" },
        );
        await vi.waitFor(() => expect(notice().title).toBe("Restored the layout from 0.59.32"));
        expect(h.reopenRestoredWindows).not.toHaveBeenCalled();
    });

    it("Not now restores nothing", async () => {
        h.OtherBuildLayoutsCommand.mockResolvedValue({ offer: true, layouts: [layout()] });
        await offerLayoutFromOtherBuild(NOW);
        press("Not now");
        expect(h.RestoreOtherBuildLayoutCommand).not.toHaveBeenCalled();
    });

    it("opens the layout's other windows once they are rebuilt", async () => {
        h.OtherBuildLayoutsCommand.mockResolvedValue({ offer: true, layouts: [layout({ windows: 2 })] });
        h.RestoreOtherBuildLayoutCommand.mockResolvedValue({
            workspace_id: "ws",
            tab_ids: ["t1"],
            replaced_default: true,
            other_windows: 1,
        });
        await offerLayoutFromOtherBuild(NOW);
        press("Restore");
        await vi.waitFor(() => expect(h.reopenRestoredWindows).toHaveBeenCalledTimes(1));
    });

    it("reports a failed restore", async () => {
        h.OtherBuildLayoutsCommand.mockResolvedValue({ offer: true, layouts: [layout()] });
        h.RestoreOtherBuildLayoutCommand.mockRejectedValue(new Error("0.59.32 has no saved layout"));
        await offerLayoutFromOtherBuild(NOW);
        press("Restore");
        await vi.waitFor(() => expect(notice().type).toBe("error"));
        expect(notice().message).toBe("0.59.32 has no saved layout");
    });
});

describe("☰ → Layouts → Restore layout from another build…", () => {
    it("lists every layout found, offered or not, each with its own button", async () => {
        h.OtherBuildLayoutsCommand.mockResolvedValue({
            offer: false,
            layouts: [layout(), layout({ channel: "stable", version: "0.59.30", saved_at_ms: NOW - 40 * 86_400_000 })],
        });
        await chooseLayoutFromOtherBuild();
        const n = notice();
        expect(n.id).toBe(NOTICE_ID);
        expect(n.actions?.map((a) => a.label)).toEqual(["Restore 0.59.32", "Restore 0.59.30", "Not now"]);
        expect(n.message.split("\n")).toHaveLength(2);
        h.RestoreOtherBuildLayoutCommand.mockResolvedValue({ workspace_id: "ws", tab_ids: ["t"], replaced_default: false, other_windows: 0 });
        press("Restore 0.59.30");
        await vi.waitFor(() =>
            expect(h.RestoreOtherBuildLayoutCommand).toHaveBeenCalledWith(
                { tag: "tab-client" },
                { window_id: "win-7", channel: "stable", version: "0.59.30" },
            ),
        );
    });

    it("says so when no other build has a layout", async () => {
        h.OtherBuildLayoutsCommand.mockResolvedValue({ offer: false, layouts: [] });
        await chooseLayoutFromOtherBuild();
        expect(notice().title).toBe("No other build has a saved layout");
        expect(notice().actions).toBeUndefined();
    });
});

describe("wording", () => {
    it("says when, honestly", () => {
        const now = new Date(NOW);
        expect(whenSaved(layout(), now)).toBe("closed 03:28 today");
        expect(whenSaved(layout({ saved_at_exact: false }), now)).toBe("last used 03:28 today");
        expect(whenSaved(layout({ saved_at_ms: new Date(2026, 9, 19, 22, 10).getTime() }), now)).toBe("closed yesterday at 22:10");
    });

    it("counts windows only when there are several, and panes when there are no agents", () => {
        expect(layoutSummary(layout())).toBe("4 tabs, 3 agents");
        expect(layoutSummary(layout({ windows: 2, tabs: 1, agents: [], panes: 2 }))).toBe("2 windows, 1 tab, 2 panes");
    });
});
