// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A hosted workspace that is going to stop shows in the status bar with the time left and why; with no stop pending it shows nothing.

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

type Stop = { stopAtMs: number; reasonId: string; reason: string } | null;
const h = vi.hoisted(() => ({ state: null as null | ReturnType<typeof import("solid-js").createSignal<Stop>>, started: vi.fn() }));
vi.mock("@/app/store/workspace-stop", () => ({
    workspaceStop: () => h.state![0](),
    startWorkspaceStop: () => h.started(),
    countdown: (at: number, now: number) => `${Math.max(0, Math.ceil((at - now) / 1000))}s`,
    clockTime: () => "14:32",
}));

import { WorkspaceStopStatus } from "./WorkspaceStopStatus";

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
        h.state[1]({ stopAtMs: Date.now() + 600_000, reasonId: "timer", reason: "nothing has happened for 50 minutes" });
        const item = container.querySelector(".status-bar-item") as HTMLElement;
        expect(item).not.toBeNull();
        expect(item.textContent).toMatch(/^Stops in \d+s$/);
        const tip = item.getAttribute("data-tip") ?? "";
        expect(tip).toContain("14:32");
        expect(tip).toContain("nothing has happened for 50 minutes");
        expect(tip).toContain("Your data is saved");
        expect(item.getAttribute("aria-label")).toBe(tip);
        expect(screen.getByRole("status")).toBe(item);
    });

    it("goes away when the stop is lifted", () => {
        h.state = createSignal<Stop>({ stopAtMs: Date.now() + 60_000, reasonId: "person", reason: "you asked" });
        const { container } = render(() => <WorkspaceStopStatus />);
        expect(container.querySelector(".status-bar-item")).not.toBeNull();
        h.state[1](null);
        expect(container.querySelector(".status-bar-item")).toBeNull();
    });
});
