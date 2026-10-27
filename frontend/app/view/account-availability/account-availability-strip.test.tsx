// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The account episode strip and its move (docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6.2, §6.3). */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AccountAvailability } from "@/types/rpc/AccountAvailability";

const { availability, move, listeners } = vi.hoisted(() => ({
    availability: vi.fn(),
    move: vi.fn(),
    listeners: [] as (() => void)[],
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { IdentityAccountAvailabilityCommand: availability, IdentityAccountMoveAgentsCommand: move },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_e: unknown, cb: () => void) => {
        listeners.push(cb);
        return () => listeners.splice(listeners.indexOf(cb), 1);
    },
}));

import { episodeLine, episodeReason, moveOutcome, preselectedCandidate } from "@/app/store/account-availability";
import { AccountAvailabilityStrip } from "./account-availability-strip";

const episode = (over: Partial<AccountAvailability> = {}): AccountAvailability => ({
    account_id: "acct-1",
    account_label: "Work",
    provider: "claude",
    state: "limited",
    resets_at_ms: null,
    limit_window: "seven_day",
    since_ms: 0,
    agents: [
        { block_id: "b1", agent_id: "d1", agent_name: "Naki-2" },
        { block_id: "b2", agent_id: "d2", agent_name: "Loap-3" },
    ],
    tasks: [
        { id: "t1", number: 163, title: "x" },
        { id: "t2", number: 140, title: "y" },
    ],
    candidates: [
        { account_id: "unchecked", label: "Spare", status: "unknown" },
        { account_id: "home", label: "Home", status: "valid" },
    ],
    agent_ids: ["d1", "d2"],
    block_ids: ["b1", "b2"],
    ...over,
});

afterEach(() => {
    cleanup();
    availability.mockReset();
    move.mockReset();
});

describe("the account episode's words", () => {
    it("reads as the spec's line", () => {
        expect(episodeLine(episode())).toBe("Account Work: out of quota · Naki-2, Loap-3 · holding #163, #140");
        expect(episodeLine(episode({ tasks: [] }))).toBe("Account Work: out of quota · Naki-2, Loap-3");
        expect(episodeReason({ state: "signed_out", resets_at_ms: null })).toBe("needs you to sign in again");
        expect(episodeReason({ state: "limited", resets_at_ms: Date.UTC(2026, 9, 25, 19) })).toMatch(/^out of quota until /);
    });

    it("preselects the first signed-in account, else the first", () => {
        expect(preselectedCandidate(episode())).toBe("home");
        expect(preselectedCandidate(episode({ candidates: [{ account_id: "a", label: "A", status: "unknown" }] }))).toBe("a");
        expect(preselectedCandidate(episode({ candidates: [] }))).toBe("");
    });

    it("says what moved, what waits for its turn, and what failed", () => {
        expect(
            moveOutcome({
                moved: [{ block_id: "b1", agent_id: "d1", agent_name: "Naki-2" }],
                busy: [{ block_id: "b2", agent_id: "d2", agent_name: "Loap-3" }],
                failed: [{ block_id: "b3", agent_name: "", error: "not an open agent pane" }],
            })
        ).toBe("Moved Naki-2. Loap-3: in a turn; moves when the turn ends. A pane: not an open agent pane");
    });
});

describe("AccountAvailabilityStrip", () => {
    it("shows nothing while no account is blocked", async () => {
        availability.mockResolvedValue({ episodes: [] });
        const { container } = render(() => <AccountAvailabilityStrip />);
        await waitFor(() => expect(availability).toHaveBeenCalled());
        expect(container.querySelector(".account-strip")).toBeNull();
    });

    it("asks which account, preselected, and moves the episode's agents there", async () => {
        availability.mockResolvedValue({ episodes: [episode()] });
        move.mockResolvedValue({ moved: [{ block_id: "b1", agent_id: "d1", agent_name: "Naki-2" }], busy: [{ block_id: "b2", agent_id: "d2", agent_name: "Loap-3" }], failed: [] });
        render(() => <AccountAvailabilityStrip />);
        expect(await screen.findByText(/Account Work: out of quota/)).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: /Move these agents to another signed-in account/ }));
        const picker = screen.getByRole("combobox", { name: "Account to move the agents to" }) as HTMLSelectElement;
        expect(picker.value).toBe("home");
        expect([...picker.options].map((o) => o.textContent)).toEqual(["Spare (not checked yet)", "Home (signed in)"]);
        fireEvent.click(screen.getByRole("button", { name: "Move" }));
        await waitFor(() => expect(move).toHaveBeenCalledWith({}, { from: "acct-1", to: "home", block_ids: ["b1", "b2"] }));
        expect(await screen.findByText(/Moved Naki-2\. Loap-3: in a turn/)).toBeTruthy();
    });

    it("says so when there is no account to move to, and follows srv's changes", async () => {
        availability.mockResolvedValue({ episodes: [episode({ candidates: [], state: "signed_out" })] });
        const { container } = render(() => <AccountAvailabilityStrip />);
        expect(await screen.findByText("No other signed-in account to move them to")).toBeTruthy();
        expect(screen.getByText(/needs you to sign in again/)).toBeTruthy();
        availability.mockResolvedValue({ episodes: [] });
        listeners.forEach((cb) => cb());
        await waitFor(() => expect(container.querySelector(".account-strip")).toBeNull(), { timeout: 2000 });
    });
});
