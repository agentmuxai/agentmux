// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// "Pause all agent browsing" shows in the status bar while it is on, says
// who paused it so nobody takes it for a fault, and a click resumes.

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

type PauseAll = { paused: boolean; since_ms: number | null; by: string | null };
const h = vi.hoisted(() => ({
    state: null as null | ReturnType<typeof import("solid-js").createSignal<PauseAll>>,
    set: vi.fn(),
}));
vi.mock("@/app/store/browser-pause-all", () => ({
    browserPauseAll: () => h.state![0](),
    pausedLabel: (p: { by: string | null }) => `Agent browsing paused by ${p.by ?? "you"}`,
    startBrowserPauseAll: () => {},
    setAgentBrowsingPaused: (paused: boolean) => {
        h.set(paused);
        h.state![1]({ paused, since_ms: null, by: null });
        return Promise.resolve();
    },
}));

import { BrowserPauseAllStatus } from "./BrowserPauseAllStatus";

afterEach(cleanup);

describe("BrowserPauseAllStatus", () => {
    it("shows nothing while agents may browse", () => {
        h.state = createSignal<PauseAll>({ paused: false, since_ms: null, by: null });
        render(() => <BrowserPauseAllStatus />);
        expect(screen.queryByRole("button")).toBeNull();
    });

    it("names the agent that paused it when an agent did", () => {
        h.state = createSignal<PauseAll>({ paused: true, since_ms: 1, by: "naki-2" });
        render(() => <BrowserPauseAllStatus />);
        expect(screen.getByText("Agent browsing paused by naki-2. Resume")).toBeTruthy();
    });

    it("says it was you who paused, and a click resumes", async () => {
        h.state = createSignal<PauseAll>({ paused: true, since_ms: 1, by: null });
        render(() => <BrowserPauseAllStatus />);
        expect(screen.getByText("Agent browsing paused by you. Resume")).toBeTruthy();
        expect(screen.getByRole("button").getAttribute("data-tip")).toContain("Click to resume");
        fireEvent.click(screen.getByRole("button"));
        await waitFor(() => expect(h.set).toHaveBeenCalledWith(false));
        await waitFor(() => expect(screen.queryByRole("button")).toBeNull());
    });
});
