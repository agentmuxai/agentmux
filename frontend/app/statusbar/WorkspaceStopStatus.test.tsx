// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A hosted workspace that is going to stop shows in the status bar with the time left and why; with no stop pending it shows nothing.
// Clicking it offers Keep running and Stop now.

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Stop = { stopAtMs: number; reasonId: string; reason: string } | null;
const h = vi.hoisted(() => ({
    state: null as null | ReturnType<typeof import("solid-js").createSignal<Stop>>,
    started: vi.fn(),
    keep: vi.fn(),
    stop: vi.fn(),
}));
vi.mock("@/app/store/workspace-stop", () => ({
    workspaceStop: () => h.state![0](),
    startWorkspaceStop: () => h.started(),
    countdown: (at: number, now: number) => `${Math.max(0, Math.ceil((at - now) / 1000))}s`,
    clockTime: () => "14:32",
}));
vi.mock("@/app/store/workspace-stop-actions", () => ({ keepRunning: () => h.keep(), stopNow: () => h.stop() }));

import { WorkspaceStopStatus } from "./WorkspaceStopStatus";

const pending = (): Stop => ({ stopAtMs: Date.now() + 600_000, reasonId: "timer", reason: "nothing has happened for 50 minutes" });

beforeEach(() => {
    h.keep.mockReset();
    h.stop.mockReset();
    h.keep.mockResolvedValue({ ok: true });
    h.stop.mockResolvedValue({ ok: true });
});
afterEach(cleanup);

describe("WorkspaceStopStatus", () => {
    it("shows nothing while no stop is pending, and starts following the event", () => {
        h.state = createSignal<Stop>(null);
        const { container } = render(() => <WorkspaceStopStatus />);
        expect(container.querySelector(".status-bar-item")).toBeNull();
        expect(h.started).toHaveBeenCalled();
    });

    it("says how long is left, and when and why in its tip, once a stop is pending", () => {
        h.state = createSignal<Stop>(null);
        const { container } = render(() => <WorkspaceStopStatus />);
        h.state[1](pending());
        const item = container.querySelector(".status-bar-item") as HTMLElement;
        expect(item).not.toBeNull();
        expect(item.textContent).toMatch(/^Stops in \d+s$/);
        const tip = item.getAttribute("data-tip") ?? "";
        expect(tip).toContain("14:32");
        expect(tip).toContain("nothing has happened for 50 minutes");
        expect(tip).toContain("Your data is saved");
        expect(item.getAttribute("aria-label")).toBe(tip);
        expect(screen.getByRole("button", { name: tip })).toBe(item);
    });

    it("goes away when the stop is lifted", () => {
        h.state = createSignal<Stop>(pending());
        const { container } = render(() => <WorkspaceStopStatus />);
        expect(container.querySelector(".status-bar-item")).not.toBeNull();
        h.state[1](null);
        expect(container.querySelector(".status-bar-item")).toBeNull();
    });
});

describe("the choices it offers", () => {
    const openPanel = () => {
        h.state = createSignal<Stop>(pending());
        const view = render(() => <WorkspaceStopStatus />);
        fireEvent.click(view.container.querySelector(".status-bar-item")!);
        return view;
    };

    it("opens a panel with Keep running and Stop now when the chip is pressed, and offers nothing before", () => {
        h.state = createSignal<Stop>(pending());
        const { container } = render(() => <WorkspaceStopStatus />);
        expect(screen.queryByRole("button", { name: "Keep running" })).toBeNull();
        fireEvent.click(container.querySelector(".status-bar-item")!);
        expect(screen.getByRole("dialog")).toBeTruthy();
        expect(screen.getByRole("button", { name: "Keep running" })).toBeTruthy();
        expect(screen.getByRole("button", { name: "Stop now" })).toBeTruthy();
        expect((container.querySelector(".status-bar-item") as HTMLElement).getAttribute("aria-expanded")).toBe("true");
    });

    it("opens from the keyboard too", () => {
        h.state = createSignal<Stop>(pending());
        const { container } = render(() => <WorkspaceStopStatus />);
        fireEvent.keyDown(container.querySelector(".status-bar-item")!, { key: "Enter" });
        expect(screen.getByRole("dialog")).toBeTruthy();
    });

    it("moves the keyboard into the panel when it opens, and back to the chip on Esc", async () => {
        const { container } = openPanel();
        await waitFor(() => expect((document.activeElement as HTMLElement | null)?.textContent).toBe("Keep running"));
        fireEvent.keyDown(document, { key: "Escape" });
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
        expect(document.activeElement).toBe(container.querySelector(".status-bar-item"));
    });

    it("keeps the workspace running on Keep running, and closes the panel", async () => {
        openPanel();
        fireEvent.click(screen.getByRole("button", { name: "Keep running" }));
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
        expect(h.keep).toHaveBeenCalledTimes(1);
        expect(h.stop).not.toHaveBeenCalled();
    });

    it("stops it now on Stop now", async () => {
        openPanel();
        fireEvent.click(screen.getByRole("button", { name: "Stop now" }));
        await waitFor(() => expect(h.stop).toHaveBeenCalledTimes(1));
        expect(h.keep).not.toHaveBeenCalled();
    });

    it("says why when it did not work, and leaves the panel open to try again", async () => {
        h.keep.mockResolvedValue({ ok: false, message: "That workspace is already stopping." });
        openPanel();
        fireEvent.click(screen.getByRole("button", { name: "Keep running" }));
        expect((await screen.findByRole("alert")).textContent).toContain("already stopping");
        expect(screen.getByRole("dialog")).toBeTruthy();
    });
});
