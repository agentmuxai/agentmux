// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { agentMountForgotten, agentMountSettled, agentMountStalled, MOUNT_WATCH_MS, mountWatchEnv } from "./agent-mount-watch";
import { useAgentMountWatch, type StackMembers } from "./pane-leaf-state";

const ID = "5c1e0a11-aaaa";

/** One member whose view type and chosen agent the test changes, as block meta updates do. */
function pane(view: string, hasSession: boolean) {
    const [viewType, setView] = createSignal(view);
    const [session, setSession] = createSignal(hasSession);
    const members: StackMembers = {
        stackBlockIds: () => [ID],
        viewTypeOf: () => viewType(),
        hasSessionOf: () => session(),
        mountedBlockIds: () => [],
    };
    return { members, setView, setSession };
}

describe("the agent mount watch of a pane", () => {
    const original = { ...mountWatchEnv };
    let lines: string[];
    let dispose: () => void;

    beforeEach(() => {
        vi.useFakeTimers();
        lines = [];
        mountWatchEnv.now = () => Date.now();
        mountWatchEnv.log = (line) => lines.push(line);
    });

    afterEach(() => {
        dispose?.();
        agentMountForgotten(ID);
        vi.useRealTimers();
        Object.assign(mountWatchEnv, original);
    });

    const watch = (members: StackMembers) =>
        createRoot((d) => {
            dispose = d;
            useAgentMountWatch({ isMinimized: () => false } as never, () => ID, members);
        });
    const wait = (ms = MOUNT_WATCH_MS + 1) => vi.advanceTimersByTime(ms);

    it("does not raise the banner on the picker, where no agent is chosen", () => {
        watch(pane("agent", false).members);
        wait(MOUNT_WATCH_MS * 3);
        expect(agentMountStalled(ID)).toBe(false);
        expect(lines).toEqual([]);
    });

    it("raises the banner for an agent pane whose view never mounts", () => {
        watch(pane("agent", true).members);
        wait();
        expect(agentMountStalled(ID)).toBe(true);
        expect(lines).toHaveLength(1);
    });

    it("raises it for a pane whose block never loads", () => {
        watch(pane("", false).members);
        wait();
        expect(agentMountStalled(ID)).toBe(true);
    });

    it("does not raise it once the view mounts", () => {
        watch(pane("agent", true).members);
        wait(5_000);
        agentMountSettled(ID);
        wait(MOUNT_WATCH_MS * 2);
        expect(agentMountStalled(ID)).toBe(false);
    });

    it("starts waiting when an agent is chosen from the picker, and stops when it goes back", () => {
        const p = pane("agent", false);
        watch(p.members);
        wait(MOUNT_WATCH_MS * 2);
        expect(agentMountStalled(ID)).toBe(false);

        p.setSession(true);
        wait(5_000);
        p.setSession(false);
        wait(MOUNT_WATCH_MS * 2);
        expect(agentMountStalled(ID)).toBe(false);

        p.setSession(true);
        wait();
        expect(agentMountStalled(ID)).toBe(true);
    });

    it("clears a banner that was up when the pane returns to the picker", () => {
        const p = pane("agent", true);
        watch(p.members);
        wait();
        expect(agentMountStalled(ID)).toBe(true);
        p.setSession(false);
        expect(agentMountStalled(ID)).toBe(false);
    });

    it("leaves a member that is not an agent unwatched", () => {
        watch(pane("help", false).members);
        wait(MOUNT_WATCH_MS * 2);
        expect(agentMountStalled(ID)).toBe(false);
    });
});
