// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How much of each account's usage window is used (`identity.account.usage`, from the provider's own
// report; `backend::account_usage`). One shared copy, fetched while any row is mounted: now, then every
// minute (a reading changes when an agent's turn ends).

import { createSignal, onCleanup, type Accessor } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { resetLabel } from "@/util/reset-label";
import type { AccountUsage } from "@/types/rpc/AccountUsage";
import type { AccountView } from "@/types/rpc/AccountView";
import type { UsageWindow } from "@/types/rpc/UsageWindow";

const POLL_MS = 60_000;
/** A reading older than this is shown with its age: no frame has come since. */
const STALE_MS = 15 * 60_000;
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

/** "5-hour", "7-day", "7-day Opus" (`seven_day_opus`), or the provider's own name with its underscores undone. */
export function windowName(key: string): string {
    const m = /^(five_hour|seven_day)(?:_(.+))?$/.exec(key);
    if (!m) return key.replace(/_/g, " ");
    const base = m[1] === "five_hour" ? "5-hour" : "7-day";
    if (!m[2]) return base;
    const rest = m[2].replace(/_/g, " ");
    return `${base} ${rest.charAt(0).toUpperCase()}${rest.slice(1)}`;
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

/** "60% used · resets Sat 7:00 PM". */
export function windowText(w: UsageWindow): string {
    return `${Math.round(w.used_percent)}% used · resets ${resetLabel(w.resets_at_ms)}`;
}

/** " · as of 20 min ago" (with its separator) for a reading no frame has refreshed in a while, else nothing. */
export function ageNote(w: Pick<UsageWindow, "age_ms">): string {
    if (w.age_ms < STALE_MS) return "";
    const min = Math.round(w.age_ms / 60_000);
    return min < 120 ? ` · as of ${min} min ago` : ` · as of ${Math.round(min / 60)} h ago`;
}

/** What to say when an account has no window to show. */
export function noReadingText(usage: AccountUsage): string {
    return usage.status === "not_reported" ? "Usage isn't reported for this provider" : "No reading yet: it comes with the next turn";
}

/** The accounts a strip shows: those with a reading. */
export function accountsWithReading(all: AccountView[]): AccountView[] {
    return all.filter((a) => a.usage.status === "reported" && Object.keys(a.usage.windows).length > 0);
}
