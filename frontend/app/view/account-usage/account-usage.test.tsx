// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The account usage rows: the strip at the top of the Swarm and the field in an account's panel. */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AccountView } from "@/types/rpc/AccountView";
import type { UsageWindow } from "@/types/rpc/UsageWindow";

const { usage } = vi.hoisted(() => ({ usage: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { IdentityAccountUsageCommand: usage } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { ageNote, barPercent, usageTone, windowName, windowText } from "@/app/store/account-usage";
import { AccountUsageField, AccountUsageStrip } from "./account-usage";

const window_ = (over: Partial<UsageWindow> = {}): UsageWindow => ({
    used_percent: 60,
    resets_at_ms: Date.now() + 3_600_000,
    observed_at_ms: Date.now(),
    age_ms: 5_000,
    ...over,
});

const account = (id: string, usage_: AccountView["usage"]): AccountView =>
    ({
        account_id: id,
        provider: "claude",
        email: `${id}@example.com`,
        label: `Account ${id}`,
        sign_in: "valid",
        usage: usage_,
        never_failover: false,
        agents: [],
    }) as unknown as AccountView;

afterEach(() => {
    cleanup();
    usage.mockReset();
});

describe("the words and the bar", () => {
    it("names the provider's windows, clamps the bar and turns it amber then red at 75 and 90", () => {
        expect([windowName("five_hour"), windowName("seven_day"), windowName("seven_day_opus")]).toEqual(["5-hour", "7-day", "seven day opus"]);
        expect([barPercent({ used_percent: -3 }), barPercent({ used_percent: 41.6 }), barPercent({ used_percent: 130 })]).toEqual([0, 42, 100]);
        expect([74.9, 75, 89.9, 90].map((p) => usageTone({ used_percent: p }))).toEqual(["ok", "warn", "warn", "high"]);
        expect(windowText(window_({ used_percent: 60.4 }))).toMatch(/^60% used · resets /);
    });

    it("shows a reading's age only once no frame has refreshed it for a while", () => {
        expect(ageNote({ age_ms: 5 * 60_000 })).toBe("");
        expect(ageNote({ age_ms: 30 * 60_000 })).toBe("as of 30 min ago");
        expect(ageNote({ age_ms: 3 * 3_600_000 })).toBe("as of 3 h ago");
    });
});

describe("the Swarm strip", () => {
    it("lists the accounts that report, one line per window, and leaves out those that don't", async () => {
        usage.mockResolvedValue({
            accounts: [
                account("work", {
                    status: "reported",
                    windows: {
                        seven_day: window_({ used_percent: 44, age_ms: 40 * 60_000, resets_at_ms: Date.now() + 86_400_000 }),
                        five_hour: window_({ used_percent: 91, resets_at_ms: Date.now() + 3_600_000 }),
                    },
                }),
                account("cx", { status: "not_reported", windows: {} }),
                account("fresh", { status: "not_yet", windows: {} }),
            ],
        });
        render(() => <AccountUsageStrip />);
        await waitFor(() => expect(screen.getByText("Account work")).toBeTruthy());
        expect(screen.queryByText("Account cx")).toBeNull();
        expect(screen.queryByText("Account fresh")).toBeNull();
        const bars = screen.getAllByRole("progressbar");
        expect(bars.map((b) => b.getAttribute("aria-valuenow"))).toEqual(["91", "44"]);
        expect(bars[0].closest("li")?.getAttribute("data-tone")).toBe("high");
        expect(bars[1].closest("li")?.getAttribute("data-tone")).toBe("ok");
        expect(screen.getByText(/as of 40 min ago/)).toBeTruthy();
    });

    it("shows nothing when no account reports", async () => {
        usage.mockResolvedValue({ accounts: [account("cx", { status: "not_reported", windows: {} })] });
        const { container } = render(() => <AccountUsageStrip />);
        await waitFor(() => expect(usage).toHaveBeenCalled());
        expect(container.querySelector(".account-usage-strip")).toBeNull();
    });
});

describe("an account's panel", () => {
    it("says plainly when a provider reports no usage or none has come yet, and shows nothing for an unknown account", async () => {
        usage.mockResolvedValue({
            accounts: [account("cx", { status: "not_reported", windows: {} }), account("fresh", { status: "not_yet", windows: {} })],
        });
        const cx = render(() => <AccountUsageField accountId="cx" />);
        await waitFor(() => expect(screen.getByText("Usage isn't reported for this provider")).toBeTruthy());
        cx.unmount();
        const fresh = render(() => <AccountUsageField accountId="fresh" />);
        await waitFor(() => expect(screen.getByText(/No reading yet/)).toBeTruthy());
        fresh.unmount();
        const none = render(() => <AccountUsageField accountId="elsewhere" />);
        expect(none.container.textContent).toBe("");
    });
});
