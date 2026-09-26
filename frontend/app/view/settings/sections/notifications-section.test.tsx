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
 *   entry renders the row as unavailable instead of a live toggle;
 * - a host without a tray or login entries (HostCaps) doesn't show those rows.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.fn();
const notifyTest = vi.fn();
const getAutostartStatus = vi.fn();
let settings: Record<string, unknown> = {};
// Reading this inside the mocked atom makes it reactive, like the real one:
// `bump()` stands in for a settings broadcast replacing the whole object.
const [settingsVersion, setSettingsVersion] = createSignal(0);
const bump = () => setSettingsVersion((v) => v + 1);

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetConfigCommand: (...args: unknown[]) => setConfig(...args),
        NotifyTestCommand: (...args: unknown[]) => notifyTest(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ settingsAtom: () => (settingsVersion(), { ...settings }) }));

import { makeTestHostApi } from "@/app/host/test-host";
import { NotificationsSection } from "./notifications-section";

/** The desktop host: has a tray and login entries. */
function useHost(caps: Partial<HostCaps> = { tray: true, autostart: true }) {
    window.api = makeTestHostApi({ getAutostartStatus: () => getAutostartStatus() }, caps);
}

function toggleFor(label: string): HTMLElement {
    const row = screen.getByText(label).closest(".setting-row");
    expect(row).not.toBeNull();
    return row!.querySelector('[role="switch"]') as HTMLElement;
}

describe("Notifications & Tray — run in background", () => {
    beforeEach(() => {
        setConfig.mockReset();
        getAutostartStatus.mockReset();
        useHost();
        getAutostartStatus.mockResolvedValue({ available: true, enabled: false });
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
        getAutostartStatus.mockReset();
        useHost();
        settings = {};
    });
    afterEach(() => cleanup());

    it("is off by default and writes app:startatlogin=true when turned on", async () => {
        getAutostartStatus.mockResolvedValue({ available: true, enabled: false });
        render(() => <NotificationsSection />);
        await waitFor(() => expect(getAutostartStatus).toHaveBeenCalled());
        const t = toggleFor("Start at login");
        expect(t.getAttribute("aria-checked")).toBe("false");
        fireEvent.click(t);
        expect(setConfig.mock.calls[0][1]).toEqual({ "app:startatlogin": true });
    });

    it("shows the setting, not the registration, so it matches the tray", async () => {
        settings = { "app:startatlogin": true };
        getAutostartStatus.mockResolvedValue({ available: true, enabled: true });
        render(() => <NotificationsSection />);
        await waitFor(() => expect(getAutostartStatus).toHaveBeenCalled());
        expect(toggleFor("Start at login").getAttribute("aria-checked")).toBe("true");
        fireEvent.click(toggleFor("Start at login"));
        expect(setConfig.mock.calls[0][1]).toEqual({ "app:startatlogin": false });
    });

    it("says so when the setting is on but nothing is registered", async () => {
        settings = { "app:startatlogin": true };
        getAutostartStatus.mockResolvedValue({ available: true, enabled: false });
        render(() => <NotificationsSection />);
        await waitFor(() => expect(screen.getByText(/not registered with the system yet/)).toBeTruthy());
    });

    it("re-reads the registration only when start at login itself changes", async () => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        try {
            getAutostartStatus.mockResolvedValue({ available: true, enabled: false });
            render(() => <NotificationsSection />);
            await waitFor(() => expect(getAutostartStatus).toHaveBeenCalledTimes(1));

            // An unrelated setting changes: the atom is replaced, nothing re-reads.
            settings = { "term:fontsize": 13 };
            bump();
            await vi.advanceTimersByTimeAsync(5000);
            expect(getAutostartStatus).toHaveBeenCalledTimes(1);

            // This setting changes: one re-read, after the settle delay.
            settings = { "term:fontsize": 13, "app:startatlogin": true };
            bump();
            await vi.advanceTimersByTimeAsync(5000);
            expect(getAutostartStatus).toHaveBeenCalledTimes(2);
        } finally {
            vi.useRealTimers();
        }
    });

    it("renders as unavailable when the host can't reach the launcher", async () => {
        getAutostartStatus.mockRejectedValue(new Error("unknown command"));
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
        getAutostartStatus.mockReset().mockResolvedValue({ available: true, enabled: false });
        useHost();
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

describe("Notifications & Tray — a host without a tray or login entries", () => {
    beforeEach(() => {
        getAutostartStatus.mockReset().mockResolvedValue({ available: false, enabled: false });
        settings = {};
    });
    afterEach(() => cleanup());

    it("shows neither row, nor the section header", () => {
        useHost({});
        render(() => <NotificationsSection />);
        expect(screen.queryByText("Keep running in the system tray")).toBeNull();
        expect(screen.queryByText("Start at login")).toBeNull();
        expect(screen.queryByText("System tray")).toBeNull();
        // The rest of the section is still there.
        expect(screen.getByText("Desktop notifications")).toBeTruthy();
    });

    it("shows only what the host has", () => {
        useHost({ tray: true });
        render(() => <NotificationsSection />);
        expect(screen.getByText("Keep running in the system tray")).toBeTruthy();
        expect(screen.queryByText("Start at login")).toBeNull();
        expect(screen.getByText("System tray")).toBeTruthy();
    });
});
