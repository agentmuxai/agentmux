// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The account usage rows: the field in an account's panel. */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AccountView } from "@/types/rpc/AccountView";
import type { UsageWindow } from "@/types/rpc/UsageWindow";

const { usage } = vi.hoisted(() => ({ usage: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { IdentityAccountUsageCommand: usage } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { ageText, barPercent, usageTone, windowName, windowText } from "@/app/store/account-usage";
import * as accountUsageModule from "./account-usage";
import { AccountUsageField } from "./account-usage";

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
        expect([windowName("five_hour"), windowName("seven_day"), windowName("seven_day_opus"), windowName("seven_day_sonnet")]).toEqual([
            "5-hour",
            "7-day",
            "7-day Opus",
            "7-day Sonnet",
        ]);
        expect([barPercent({ used_percent: -3 }), barPercent({ used_percent: 41.6 }), barPercent({ used_percent: 130 })]).toEqual([0, 42, 100]);
        expect([74.9, 75, 89.9, 90].map((p) => usageTone({ used_percent: p }))).toEqual(["ok", "warn", "warn", "high"]);
        expect(windowText(window_({ used_percent: 60.4 }))).toMatch(/^60% used · resets /);
    });

    it("gives the model's weekly window its plain name and never dresses an unknown one as a plan window", () => {
        expect(windowName("seven_day_overage_included")).toBe("7-day Fable");
        expect(windowName("monthly_extra")).toBe("monthly extra");
        expect(windowName("seven_day_cowork")).toBe("seven day cowork");
    });

    it("adds the date to a reset more than a day away, and only then", () => {
        const now = new Date(2026, 10, 21, 20, 0).getTime();
        const soon = windowText(window_({ resets_at_ms: now + 5 * 3_600_000 }), now);
        const later = windowText(window_({ resets_at_ms: now + 7 * 86_400_000 }), now);
        expect(soon).not.toContain("28");
        expect(later).toContain("28");
    });

    it("says how old a reading is, at any age", () => {
        expect([0, 20_000, 5 * 60_000, 90 * 60_000, 3 * 3_600_000].map(ageText)).toEqual(["just now", "just now", "5 min ago", "90 min ago", "3 h ago"]);
    });
});

describe("the account usage is not at the top of the Swarm", () => {
    it("no longer has a strip to mount", () => {
        expect("AccountUsageStrip" in accountUsageModule).toBe(false);
    });

    it("never cuts a window's name with an ellipsis", () => {
        const css = readFileSync(join(__dirname, "account-usage.scss"), "utf8");
        expect(css).not.toMatch(/text-overflow/);
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

    it("lists the windows soonest first with the model's weekly one named, and says where the numbers come from and how old they are", async () => {
        usage.mockResolvedValue({
            accounts: [
                account("work", {
                    status: "reported",
                    windows: {
                        seven_day_overage_included: window_({ used_percent: 25, age_ms: 40_000 }),
                        five_hour: window_({ used_percent: 91, resets_at_ms: Date.now() + 1_000_000, age_ms: 40_000 }),
                    },
                }),
            ],
        });
        render(() => <AccountUsageField accountId="work" />);
        await waitFor(() => expect(screen.getByText("7-day Fable")).toBeTruthy());
        const bars = screen.getAllByRole("progressbar");
        expect(bars.map((b) => b.getAttribute("aria-valuenow"))).toEqual(["91", "25"]);
        expect(bars[0].closest("li")?.getAttribute("data-tone")).toBe("high");
        expect(screen.getByText("From the provider's own report · updated just now")).toBeTruthy();
    });
});
