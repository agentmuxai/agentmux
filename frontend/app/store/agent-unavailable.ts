// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether an agent can't start a turn, and the words for it: out of quota until a
 * reset, or signed out. srv derives these as the `limited` and `signed_out` states
 * (`agent_state.rs`); a pane's own renderer gets the same answer from the failure
 * srv stored on its block (`agent:last_failure`), by the same rule, so a Swarm row
 * needs no call for it.
 * docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §3.2, §4.4.
 */

import type { UnavailableReason } from "./agent-status";

/** The block meta key srv stores the last classified failure under. */
export const META_LAST_FAILURE = "agent:last_failure";

export interface AgentUnavailable {
    reason: UnavailableReason;
    /** When the limit lifts (Unix ms), if the provider said. */
    resetsAtMs?: number;
    /** The provider's name for the limit's window, such as `five_hour` or `seven_day`. */
    limitWindow?: string;
}

/**
 * Why the agent can't start a turn, or `null` when it can. A turn in flight wins: it is
 * evidence the agent can work. A usage limit counts until its reset passes (with no
 * reset known, until something clears the failure); a refused sign-in counts until
 * something clears it. Other failures are transient and don't count.
 */
export function unavailableOfMeta(
    meta: Record<string, unknown> | null | undefined,
    live: boolean,
    now: number
): AgentUnavailable | null {
    if (live) return null;
    const failure = meta?.[META_LAST_FAILURE];
    if (!failure || typeof failure !== "object") return null;
    const { code, resetsAtMs, limitWindow } = failure as {
        code?: unknown;
        resetsAtMs?: unknown;
        limitWindow?: unknown;
    };
    if (code === "auth") return { reason: "signed_out" };
    if (code !== "usage_limit") return null;
    const resets =
        typeof resetsAtMs === "number" && Number.isFinite(resetsAtMs) && resetsAtMs > 0 ? resetsAtMs : undefined;
    if (resets !== undefined && resets <= now) return null;
    return {
        reason: "limited",
        ...(resets !== undefined ? { resetsAtMs: resets } : {}),
        ...(typeof limitWindow === "string" && limitWindow ? { limitWindow } : {}),
    };
}

const DAY_MS = 24 * 60 * 60 * 1000;

/** `7 PM`, or `7:30 PM` when it is not on the hour. */
function clock(date: Date): string {
    return date.toLocaleTimeString(undefined, {
        hour: "numeric",
        ...(date.getMinutes() ? { minute: "2-digit" } : {}),
    });
}

/**
 * When a limit lifts, in the user's own clock: `7 PM` if it is today, `Sun 7 PM` within the
 * next six days, and `Oct 26, 7 PM` after that.
 */
export function formatResetTime(resetsAtMs: number, now: number): string {
    const reset = new Date(resetsAtMs);
    if (reset.toDateString() === new Date(now).toDateString()) return clock(reset);
    if (resetsAtMs - now < 6 * DAY_MS) {
        return `${reset.toLocaleDateString(undefined, { weekday: "short" })} ${clock(reset)}`;
    }
    return `${reset.toLocaleDateString(undefined, { month: "short", day: "numeric" })}, ${clock(reset)}`;
}

/** The chip's words: `Out of quota · until Sun 7 PM`, `Out of quota`, `Signed out`. */
export function unavailableChipText(u: AgentUnavailable, now: number): string {
    if (u.reason === "signed_out") return "Signed out";
    return u.resetsAtMs ? `Out of quota · until ${formatResetTime(u.resetsAtMs, now)}` : "Out of quota";
}

/** The row line's words: `Out of quota until Sun 7 PM`, `Needs sign-in`. */
export function unavailableLineText(u: AgentUnavailable, now: number): string {
    if (u.reason === "signed_out") return "Needs sign-in";
    return u.resetsAtMs ? `Out of quota until ${formatResetTime(u.resetsAtMs, now)}` : "Out of quota";
}

/** The chip's tooltip: what the state means for what the agent can do. */
export function unavailableTitle(u: AgentUnavailable): string {
    if (u.reason === "signed_out") return "Sign-in was refused. The agent can't run until it is signed in again.";
    const window = u.limitWindow ? ` (${u.limitWindow.replace(/_/g, " ")} limit)` : "";
    return `This agent's plan is out of quota${window}. Messages to it are held until the limit lifts or it moves to another account.`;
}
