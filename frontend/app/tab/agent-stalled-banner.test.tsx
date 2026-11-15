// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const reloadWindow = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/global", () => ({ getApi: () => ({ reloadWindow }) }));

import { agentMountForgotten, agentMountSettled, expectAgentMount, MOUNT_WATCH_MS, mountWatchEnv } from "./agent-mount-watch";
import { AgentStalledBanner } from "./agent-stalled-banner";

describe("the banner for an agent pane that has not started", () => {
    const id = "7d7d7d7d-4444";
    const log = mountWatchEnv.log;
    afterEach(() => {
        cleanup();
        agentMountForgotten(id);
        mountWatchEnv.log = log;
        vi.useRealTimers();
        reloadWindow.mockClear();
    });

    it("appears after 20 s with a Reload that reloads the window, and goes when the view mounts", () => {
        vi.useFakeTimers();
        mountWatchEnv.log = () => {};
        const { container } = render(() => <AgentStalledBanner blockId={id} />);
        expectAgentMount(id, () => "");
        expect(container.querySelector(".agent-stalled-banner")).toBeNull();
        vi.advanceTimersByTime(MOUNT_WATCH_MS);
        expect(container.querySelector(".agent-stalled-banner")?.textContent).toContain("The agent is running; its history is loading.");
        fireEvent.click(screen.getByRole("button", { name: "Reload" }));
        expect(reloadWindow).toHaveBeenCalledOnce();
        agentMountSettled(id);
        expect(container.querySelector(".agent-stalled-banner")).toBeNull();
    });

    it("is not drawn for a pane that never waited", () => {
        const { container } = render(() => <AgentStalledBanner blockId="other-block" />);
        expect(container.querySelector(".agent-stalled-banner")).toBeNull();
    });
});
