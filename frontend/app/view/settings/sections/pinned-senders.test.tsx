// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings › Pinned senders: the list from `pins.list`, and the two-press Accept / Forget. */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { PinnedSendersResult } from "@/types/rpc/PinnedSendersResult";

const list = vi.fn();
const forget = vi.fn();
const adopt = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        PinsListCommand: (...args: unknown[]) => list(...args),
        PinsForgetCommand: (...args: unknown[]) => forget(...args),
        PinsAdoptCommand: (...args: unknown[]) => adopt(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { ago, PinnedSenders } from "./pinned-senders";

const NOW = Math.floor(Date.now() / 1000);

const everything: PinnedSendersResult = {
    lan: [
        { agent_id: "maop", key_fp: "aaaaaaaaaa", pinned_at: NOW - 5 * 86400, uid_pins: 1, refused: { key_fp: "bbbbbbbbbb", last_seen_at: NOW - 120, count: 3 } },
        { agent_id: "korp", key_fp: "cccccccccc", pinned_at: NOW - 3600, uid_pins: 0 },
    ],
    wan: [
        {
            agent_id: "lazo",
            channel: "stable",
            instance: "narko~abcd1234",
            pinned_at: NOW - 86400,
            refused: { instance: "area54~wxyz9876", last_seen_at: NOW - 30, count: 1 },
        },
    ],
};

beforeEach(() => {
    list.mockReset().mockResolvedValue(everything);
    forget.mockReset().mockResolvedValue({ lan: [], wan: [] });
    adopt.mockReset().mockResolvedValue({ lan: [], wan: [] });
});
afterEach(cleanup);

describe("ago", () => {
    it("reads minutes, hours and days", () => {
        expect(ago(NOW - 10, NOW)).toBe("just now");
        expect(ago(NOW - 5 * 60, NOW)).toBe("5 min ago");
        expect(ago(NOW - 3 * 3600, NOW)).toBe("3 h ago");
        expect(ago(NOW - 2 * 86400, NOW)).toBe("2 days ago");
        expect(ago(NOW + 50, NOW)).toBe("just now");
    });
});

describe("PinnedSenders", () => {
    it("lists each pin with what was refused beside it, and never a key", async () => {
        const { container } = render(() => <PinnedSenders />);
        await screen.findByText(/Key aaaaaaaaaa/);
        expect(container.querySelectorAll(".setting-pin")).toHaveLength(3);
        expect(screen.getByText(/A different key, bbbbbbbbbb, was refused 3 times/)).toBeTruthy();
        expect(screen.getByText(/area54~wxyz9876 spoke for it 1 time and was refused/)).toBeTruthy();
        expect(screen.getByText(/Pinned to narko~abcd1234/)).toBeTruthy();
    });

    it("offers Accept only where something was refused", async () => {
        render(() => <PinnedSenders />);
        await screen.findByText(/Key cccccccccc/);
        expect(screen.getAllByText("Accept the new one")).toHaveLength(2); // maop, lazo; not korp
        expect(screen.getAllByText("Forget")).toHaveLength(3);
    });

    it("needs a second press, then asks the right pin and shows the new list", async () => {
        render(() => <PinnedSenders />);
        await screen.findByText(/Key cccccccccc/);
        const buttons = screen.getAllByText("Forget");
        fireEvent.click(buttons[1]); // korp
        expect(forget).not.toHaveBeenCalled();
        fireEvent.click(await screen.findByText("Press again to forget"));
        await waitFor(() => expect(forget).toHaveBeenCalledTimes(1));
        expect(forget.mock.calls[0][1]).toEqual({ kind: "lan", agent_id: "korp", channel: undefined });
        await screen.findByText(/Nothing is pinned yet/);
    });

    it("accepts a refused WAN install for its channel", async () => {
        render(() => <PinnedSenders />);
        await screen.findByText(/Key cccccccccc/);
        const accept = screen.getAllByText("Accept the new one");
        fireEvent.click(accept[1]); // lazo
        fireEvent.click(await screen.findByText("Press again to accept"));
        await waitFor(() => expect(adopt).toHaveBeenCalledTimes(1));
        expect(adopt.mock.calls[0][1]).toEqual({ kind: "wan", agent_id: "lazo", channel: "stable" });
    });

    it("says why when the list can't be read or a change is refused", async () => {
        list.mockRejectedValueOnce(new Error("FORBIDDEN: only the user lists or changes the keys"));
        render(() => <PinnedSenders />);
        await screen.findByText(/FORBIDDEN: only the user/);
        expect(screen.queryByText("Loading…")).toBeNull();
    });
});
