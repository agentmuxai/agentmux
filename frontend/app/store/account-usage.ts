// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How much of each account's usage window is used (`identity.account.usage`, from the provider's own
// report; `backend::account_usage`). One shared copy, fetched while any row is mounted: now, then every
// minute (a reading changes when an agent's turn ends).

import { createSignal, onCleanup, type Accessor } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { resetLabelWithDate } from "@/util/reset-label";
import type { AccountUsage } from "@/types/rpc/AccountUsage";
import type { AccountView } from "@/types/rpc/AccountView";
import type { UsageWindow } from "@/types/rpc/UsageWindow";

const POLL_MS = 60_000;
/** The bar turns amber and red here, as the notes to leaders do by default. */
export const WARN_AT = 75;
export const HIGH_AT = 90;

const [accounts, setAccounts] = createSignal<AccountView[]>([]);
let users = 0;
let poll: ReturnType<typeof setInterval> | null = null;

async function refresh(): Promise<void> {
    try {
        const r = await RpcApi.IdentityAccountUsageCommand(TabRpcClient);
        setAccounts(r?.accounts ?? []);
    } catch {
        // An older srv, or none yet: no rows.
    }
}

/** The accounts with their usage, kept current while the calling component is mounted. */
export function useAccountUsage(): Accessor<AccountView[]> {
    if (users++ === 0) {
        void refresh();
        poll = setInterval(() => void refresh(), POLL_MS);
    }
    onCleanup(() => {
        if (--users === 0 && poll) {
            clearInterval(poll);
            poll = null;
        }
    });
    return accounts;
}

/**
 * The windows the provider's CLI names, in plain words. `seven_day_overage_included` is the CLI's own "Fable
 * limit": a weekly limit of one model, not the plan's weekly window.
 */
const WINDOW_NAMES: Record<string, string> = {
    five_hour: "5-hour",
    seven_day: "7-day",
    seven_day_opus: "7-day Opus",
    seven_day_sonnet: "7-day Sonnet",
    seven_day_overage_included: "7-day Fable",
};

/** A window's plain name; one we don't know is the provider's own name with its underscores undone, never dressed as a plan window. */
export function windowName(key: string): string {
    return WINDOW_NAMES[key] ?? key.replace(/_/g, " ");
}

/** The windows of a reading, the one that resets first first (`five_hour` before `seven_day`). */
export function windowsOf(usage: AccountUsage): [string, UsageWindow][] {
    return Object.entries(usage.windows).sort((a, b) => a[1].resets_at_ms - b[1].resets_at_ms);
}

/** The bar's width: 0 to 100 whatever the report says. */
export function barPercent(w: Pick<UsageWindow, "used_percent">): number {
    return Math.min(100, Math.max(0, Math.round(w.used_percent)));
}

export type UsageTone = "ok" | "warn" | "high";
export function usageTone(w: Pick<UsageWindow, "used_percent">): UsageTone {
    return w.used_percent >= HIGH_AT ? "high" : w.used_percent >= WARN_AT ? "warn" : "ok";
}

/** "60% used · resets Sat 7:00 PM"; a reset more than a day away carries its date ("resets Sat Nov 28, 1:00 AM"). */
export function windowText(w: UsageWindow, now: number = Date.now()): string {
    return `${Math.round(w.used_percent)}% used · resets ${resetLabelWithDate(w.resets_at_ms, now)}`;
}

/** How old a reading is, in a few words: "just now", "20 min ago", "3 h ago". */
export function ageText(ageMs: number): string {
    const min = Math.floor(ageMs / 60_000);
    if (min < 1) return "just now";
    return min < 120 ? `${min} min ago` : `${Math.round(min / 60)} h ago`;
}

/** The freshest age among an account's windows (a frame refreshes the windows it carries). */
export function freshestAgeMs(usage: AccountUsage): number {
    return Math.min(...Object.values(usage.windows).map((w) => w.age_ms));
}

/** What to say when an account has no window to show. */
export function noReadingText(usage: AccountUsage): string {
    return usage.status === "not_reported" ? "Usage isn't reported for this provider" : "No reading yet: it comes with the next turn";
}
