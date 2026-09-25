// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Notifications & Tray section — SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md §4.1.
 *
 * Pins the two contracts that are easy to break silently:
 * - the tray toggle writes exactly `app:runinbackground` (the key
 *   `agentmux-launcher/src/background_config.rs` reads before the app runs);
 * - auto-start goes through the host IPC verbs, and a host/launcher that
 *   can't do it renders the row as unavailable instead of a live toggle that
 *   does nothing.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.fn();
const notifyTest = vi.fn();
const invokeCommand = vi.fn();
let settings: Record<string, unknown> = {};

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetConfigCommand: (...args: unknown[]) => setConfig(...args),
        NotifyTestCommand: (...args: unknown[]) => notifyTest(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ settingsAtom: () => settings }));
vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: (...args: unknown[]) => invokeCommand(...args),
}));

import { NotificationsSection } from "./notifications-section";

function toggleFor(label: string): HTMLElement {
    const row = screen.getByText(label).closest(".setting-row");
    expect(row).not.toBeNull();
    return row!.querySelector('[role="switch"]') as HTMLElement;
}

describe("Notifications & Tray — run in background", () => {
    beforeEach(() => {
        setConfig.mockReset();
        invokeCommand.mockReset();
        invokeCommand.mockResolvedValue({ available: true, enabled: false });
        settings = {};
    });
    afterEach(() => cleanup());

    it("is off by default and writes app:runinbackground=true when turned on", () => {
        render(() => <NotificationsSection />);
        const t = toggleFor("Keep running in the system tray");
        expect(t.getAttribute("aria-checked")).toBe("false");
        fireEvent.click(t);
        expect(setConfig.mock.calls[0][1]).toEqual({ "app:runinbackground": true });
    });

    it("reflects a stored true", () => {
        settings = { "app:runinbackground": true };
        render(() => <NotificationsSection />);
        expect(toggleFor("Keep running in the system tray").getAttribute("aria-checked")).toBe("true");
    });
});

describe("Notifications & Tray — start at login", () => {
    beforeEach(() => {
        invokeCommand.mockReset();
        settings = {};
    });
    afterEach(() => cleanup());

    it("shows the launcher-reported status and toggles via set_autostart", async () => {
        invokeCommand.mockImplementation(async (cmd: string, args: any) =>
            cmd === "set_autostart" ? { available: true, enabled: args.enabled } : { available: true, enabled: false },
        );
        render(() => <NotificationsSection />);
        await waitFor(() => expect(invokeCommand).toHaveBeenCalledWith("autostart_status", {}));
        const t = toggleFor("Start at login");
        await waitFor(() => expect(t.getAttribute("aria-checked")).toBe("false"));
        fireEvent.click(toggleFor("Start at login"));
        await waitFor(() => expect(invokeCommand).toHaveBeenCalledWith("set_autostart", { enabled: true }));
        await waitFor(() => expect(toggleFor("Start at login").getAttribute("aria-checked")).toBe("true"));
    });

    it("renders as unavailable when the host can't reach the launcher", async () => {
        invokeCommand.mockRejectedValue(new Error("unknown command"));
        render(() => <NotificationsSection />);
        await waitFor(() => expect(screen.getByText(/Unavailable in this build/)).toBeTruthy());
        fireEvent.click(toggleFor("Start at login"));
        expect(invokeCommand).not.toHaveBeenCalledWith("set_autostart", expect.anything());
    });
});

describe("Notifications & Tray — desktop notifications", () => {
    beforeEach(() => {
        setConfig.mockReset();
        notifyTest.mockReset().mockResolvedValue({ ok: true });
        invokeCommand.mockReset().mockResolvedValue({ available: true, enabled: false });
        settings = {};
    });
    afterEach(() => cleanup());

    it("defaults ON and writes notify:os:enabled=false when turned off", () => {
        render(() => <NotificationsSection />);
        const t = toggleFor("Desktop notifications");
        expect(t.getAttribute("aria-checked")).toBe("true");
        fireEvent.click(t);
        expect(setConfig.mock.calls[0][1]).toEqual({ "notify:os:enabled": false });
    });

    it("hides the per-kind rows when the master switch is off", () => {
        settings = { "notify:os:enabled": false };
        render(() => <NotificationsSection />);
        expect(screen.queryByText("Agent has a question")).toBeNull();
    });

    it("per-kind toggles write their own keys", () => {
        render(() => <NotificationsSection />);
        fireEvent.click(toggleFor("Agent finished"));
        expect(setConfig.mock.calls[0][1]).toEqual({ "notify:os:turncompleted": false });
    });

    it("the summary line defaults ON and writes notify:os:summary", () => {
        render(() => <NotificationsSection />);
        const t = toggleFor("Show what the agent is working on");
        expect(t.getAttribute("aria-checked")).toBe("true");
        fireEvent.click(t);
        expect(setConfig.mock.calls[0][1]).toEqual({ "notify:os:summary": false });
    });

    it("the test button calls notify.test", () => {
        render(() => <NotificationsSection />);
        fireEvent.click(screen.getByText("Send"));
        expect(notifyTest).toHaveBeenCalledTimes(1);
    });
});
