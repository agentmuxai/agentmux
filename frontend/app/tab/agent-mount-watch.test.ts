// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { agentMountSettled, expectAgentMount, MOUNT_WATCH_MS, mountWatchEnv } from "./agent-mount-watch";

describe("agent mount watch", () => {
    let lines: string[];
    let clock: number;
    const original = { ...mountWatchEnv };

    beforeEach(() => {
        vi.useFakeTimers();
        lines = [];
        clock = 0;
        mountWatchEnv.now = () => clock;
        mountWatchEnv.log = (line) => lines.push(line);
    });

    afterEach(() => {
        agentMountSettled("0f0b7d4d-1111");
        agentMountSettled("bfbeea15-2222");
        vi.useRealTimers();
        Object.assign(mountWatchEnv, original);
    });

    const run = (ms: number) => {
        clock += ms;
        vi.advanceTimersByTime(ms);
    };

    it("logs once what is known about an agent view that never mounted", () => {
        expectAgentMount("0f0b7d4d-1111", () => "view=agent active=true mounted=false minimized=true stack=1");
        run(MOUNT_WATCH_MS - 1);
        expect(lines).toEqual([]);
        run(1);
        expect(lines).toEqual([
            "[pane-mount] block=0f0b7d4d agent view has not started after 20 s: view=agent active=true mounted=false minimized=true stack=1",
        ]);
        run(MOUNT_WATCH_MS * 3);
        expect(lines).toHaveLength(1);
    });

    it("says nothing for a view that mounts in time", () => {
        expectAgentMount("0f0b7d4d-1111", () => "unused");
        run(5_000);
        agentMountSettled("0f0b7d4d-1111");
        run(MOUNT_WATCH_MS * 2);
        expect(lines).toEqual([]);
    });

    it("watches each block on its own and starts a block's wait only once", () => {
        expectAgentMount("0f0b7d4d-1111", () => "a");
        run(10_000);
        expectAgentMount("0f0b7d4d-1111", () => "again: must not restart the wait");
        expectAgentMount("bfbeea15-2222", () => "b");
        run(10_000);
        expect(lines).toHaveLength(1);
        expect(lines[0]).toContain("block=0f0b7d4d");
        run(10_000);
        expect(lines).toHaveLength(2);
        expect(lines[1]).toContain("block=bfbeea15");
    });

    it("ignores an empty block id", () => {
        expectAgentMount("", () => "x");
        run(MOUNT_WATCH_MS * 2);
        expect(lines).toEqual([]);
    });
});
