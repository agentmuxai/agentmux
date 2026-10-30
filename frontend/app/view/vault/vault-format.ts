// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** When something happened, `ts` in unix millis: relative for today, a date before that. */
export function formatWhen(ts: number, now = Date.now()): string {
    if (!ts) return "never";
    const d = Math.max(0, now - ts);
    if (d < 60_000) return "just now";
    if (d < 3_600_000) return `${Math.floor(d / 60_000)} min ago`;
    if (d < 86_400_000) return `${Math.floor(d / 3_600_000)} h ago`;
    return new Date(ts).toLocaleDateString();
}

/** Days within which an entry counts as expiring soon: the first reminder's (14) and some. */
export const EXPIRING_SOON_DAYS = 30;

export interface Expiry {
    /** Days left: 0 is today, below 0 is past. */
    days: number;
    text: string;
    /** Past its date, or within the first reminder: shown in the warning colour. */
    warn: boolean;
}

/** An entry's `expires` field (`YYYY-MM-DD`, as srv keeps it) read against today, in local time. */
export function expiryOf(expires: string | undefined, now = new Date()): Expiry | null {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(expires?.trim() ?? "");
    if (!m) return null;
    const date = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
    if (Number.isNaN(date.getTime()) || date.getMonth() !== Number(m[2]) - 1) return null;
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    const days = Math.round((date.getTime() - today.getTime()) / 86_400_000);
    const text = days < 0 ? `expired on ${m[0]}` : days === 0 ? "expires today" : days === 1 ? "expires tomorrow" : `expires in ${days} days`;
    return { days, text, warn: days <= 14 };
}
