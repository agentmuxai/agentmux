// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The build slot note on a Swarm row: shown only on the row of the agent that holds the slot, polled
 * only while a note is mounted (SPEC_BUILD_SLOT_ENFORCEMENT_2026_10_20.md §6).
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { GetBuildSlotCommand: rpc.get } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import type { BuildSlotView } from "@/types/rpc/BuildSlotView";
import { BUILD_SLOT_POLL_MS, buildSlotText, buildSlotTitle, holdMinutes } from "./swarm-build-slot";
import { BuildSlotNote } from "./swarm-row-notes";

const NOW = 1_800_000_000_000;
const holder: BuildSlotView = {
    agent: "lazo",
    block_id: "block-lazo",
    since_ms: NOW - 12 * 60_000 - 5_000,
    reason: "cargo test",
    command: "/c/Users/x/.cargo/bin/cargo test -p agentmux-srv",
};

async function flush() {
    await Promise.resolve();
    await Promise.resolve();
}

beforeEach(() => {
    vi.useFakeTimers();
    rpc.get.mockReset();
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("build slot text", () => {
    it("counts whole minutes and drops them under the first", () => {
        expect(holdMinutes(NOW - 59_000, NOW)).toBe(0);
        expect(holdMinutes(NOW - 61_000, NOW)).toBe(1);
        expect(holdMinutes(NOW + 5_000, NOW)).toBe(0);
        expect(buildSlotText(holder, NOW)).toBe("Holding the build slot · 12 min · cargo test");
        expect(buildSlotText({ ...holder, since_ms: NOW - 5_000 }, NOW)).toBe("Holding the build slot · cargo test");
        expect(buildSlotText({ ...holder, reason: "" }, NOW)).toBe("Holding the build slot · 12 min");
    });

    it("cuts a long reason and keeps the whole command for the tooltip", () => {
        const long = { ...holder, reason: "task package:portable --force --verbose" };
        expect(buildSlotText(long, NOW)).toBe("Holding the build slot · 12 min · task package:portable -…");
        expect(buildSlotTitle(holder)).toContain("cargo test -p agentmux-srv");
        expect(buildSlotTitle({ command: "", reason: "" })).toBe("Other builds and tests on this computer wait for this one to finish.");
    });
});

describe("BuildSlotNote", () => {
    it("shows on the holder's row only", async () => {
        rpc.get.mockResolvedValue(holder);
        const mine = render(() => <BuildSlotNote blockId="block-lazo" now={() => NOW} />);
        const other = render(() => <BuildSlotNote blockId="block-other" now={() => NOW} />);
        await flush();
        expect(mine.container.textContent).toBe("Holding the build slot · 12 min · cargo test");
        expect(other.container.textContent).toBe("");
    });

    it("shows nothing when the slot is free or the holder is not a local agent", async () => {
        rpc.get.mockResolvedValue(null);
        const free = render(() => <BuildSlotNote blockId="block-lazo" now={() => NOW} />);
        await flush();
        expect(free.container.textContent).toBe("");
        free.unmount();

        rpc.get.mockResolvedValue({ ...holder, block_id: undefined });
        const stranger = render(() => <BuildSlotNote blockId="block-lazo" now={() => NOW} />);
        await flush();
        expect(stranger.container.textContent).toBe("");
        const noBlock = render(() => <BuildSlotNote blockId={null} now={() => NOW} />);
        await flush();
        expect(noBlock.container.textContent).toBe("");
    });

    it("polls while mounted, once for any number of rows, and stops when the last one goes", async () => {
        rpc.get.mockResolvedValue(null);
        const a = render(() => <BuildSlotNote blockId="a" now={() => NOW} />);
        const b = render(() => <BuildSlotNote blockId="b" now={() => NOW} />);
        await flush();
        expect(rpc.get).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(BUILD_SLOT_POLL_MS);
        expect(rpc.get).toHaveBeenCalledTimes(2);
        a.unmount();
        await vi.advanceTimersByTimeAsync(BUILD_SLOT_POLL_MS);
        expect(rpc.get).toHaveBeenCalledTimes(3);
        b.unmount();
        await vi.advanceTimersByTimeAsync(BUILD_SLOT_POLL_MS * 3);
        expect(rpc.get).toHaveBeenCalledTimes(3);
    });

    it("keeps what it had when a poll fails", async () => {
        rpc.get.mockResolvedValueOnce(holder).mockRejectedValue(new Error("down"));
        const note = render(() => <BuildSlotNote blockId="block-lazo" now={() => NOW} />);
        await flush();
        await vi.advanceTimersByTimeAsync(BUILD_SLOT_POLL_MS);
        expect(note.container.textContent).toBe("Holding the build slot · 12 min · cargo test");
    });
});
