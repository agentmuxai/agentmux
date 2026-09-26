// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Notifications & Tray section — SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md §4.1.
 *
 * Pins the two contracts that are easy to break silently:
 * - the tray toggle writes exactly `app:runinbackground` (the key
 *   `agentmux-launcher/src/background_config.rs` reads before the app runs);
 * - start at login writes exactly `app:startatlogin` (the one property the
 *   tray's check item also writes, and the launcher applies to the OS login
 *   entry), is off by default, and a host/launcher that can't manage a login
 *   entry renders the row as unavailable instead of a live toggle.
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

    it("is on by default and writes app:runinbackground=false when turned off", () => {
        render(() => <NotificationsSection />);
        const t = toggleFor("Keep running in the system tray");
        expect(t.getAttribute("aria-checked")).toBe("true");
        fireEvent.click(t);
        expect(setConfig.mock.calls[0][1]).toEqual({ "app:runinbackground": false });
    });

    it("reflects a stored true", () => {
        settings = { "app:runinbackground": true };
        render(() => <NotificationsSection />);
        expect(toggleFor("Keep running in the system tray").getAttribute("aria-checked")).toBe("true");
    });
});

describe("Notifications & Tray — start at login", () => {
    beforeEach(() => {
        setConfig.mockReset();
        invokeCommand.mockReset();
        settings = {};
    });
    afterEach(() => cleanup());

    it("is off by default and writes app:startatlogin=true when turned on", async () => {
        invokeCommand.mockResolvedValue({ available: true, enabled: false });
        render(() => <NotificationsSection />);
        await waitFor(() => expect(invokeCommand).toHaveBeenCalledWith("autostart_status", {}));
        const t = toggleFor("Start at login");
        expect(t.getAttribute("aria-checked")).toBe("false");
        fireEvent.click(t);
        expect(setConfig.mock.calls[0][1]).toEqual({ "app:startatlogin": true });
    });

    it("shows the setting, not the registration, so it matches the tray", async () => {
        settings = { "app:startatlogin": true };
        invokeCommand.mockResolvedValue({ available: true, enabled: true });
        render(() => <NotificationsSection />);
        await waitFor(() => expect(invokeCommand).toHaveBeenCalled());
        expect(toggleFor("Start at login").getAttribute("aria-checked")).toBe("true");
        fireEvent.click(toggleFor("Start at login"));
        expect(setConfig.mock.calls[0][1]).toEqual({ "app:startatlogin": false });
    });

    it("says so when the setting is on but nothing is registered", async () => {
        settings = { "app:startatlogin": true };
        invokeCommand.mockResolvedValue({ available: true, enabled: false });
        render(() => <NotificationsSection />);
        await waitFor(() => expect(screen.getByText(/not registered with the system yet/)).toBeTruthy());
    });

    it("renders as unavailable when the host can't reach the launcher", async () => {
        invokeCommand.mockRejectedValue(new Error("unknown command"));
        render(() => <NotificationsSection />);
        await waitFor(() => expect(screen.getByText(/Unavailable in this build/)).toBeTruthy());
        fireEvent.click(toggleFor("Start at login"));
        expect(setConfig).not.toHaveBeenCalled();
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
