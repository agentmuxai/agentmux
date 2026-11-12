// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an agent on a model route has spent today against its daily cap, as srv writes it to the pane's block meta
 * (`crates/srv/src/server/spend_watch.rs`, docs/specs/SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md section 3):
 *
 * - `agent:spend_today`: the figure the chips show, `{day, spent, cap}`.
 * - `agent:spend_warning`: from 80% of the cap, a line above the composer.
 * - `agent:spend_stopped`: the cap is used and the gateway refuses the agent's requests; the card offers Raise, Reset
 *   today and Dismiss.
 */

import * as MOS from "@/app/store/mos";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createSignal, onCleanup, type Accessor } from "solid-js";

export const SPEND_TODAY_META_KEY = "agent:spend_today";
export const SPEND_WARNING_META_KEY = "agent:spend_warning";
export const SPEND_STOPPED_META_KEY = "agent:spend_stopped";
/** Set by the window when the card is dismissed: the `at` of the stop it was dismissed for. The stop itself stays. */
export const SPEND_DISMISSED_META_KEY = "agent:spend_stopped_dismissed";

const DAY_MS = 86_400_000;

export type SpendScope = "agent" | "route";

export interface SpendCard {
    /** When srv wrote it, in ms. */
    at: number;
    /** Whose cap was reached: the agent's own, or its route's. */
    scope: SpendScope;
    cap: number;
    spent: number;
    route: string;
    /** "OpenRouter (DeepSeek V4.1 Flash)". */
    place: string;
}

export interface SpendToday {
    spent: number;
    /** Null when the agent has no cap. */
    cap: number | null;
}

const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);
const str = (v: unknown): string => (typeof v === "string" ? v : "");

function readCard(raw: unknown): SpendCard | null {
    if (raw == null || typeof raw !== "object") return null;
    const r = raw as Record<string, unknown>;
    const cap = num(r.cap);
    const spent = num(r.spent);
    if (cap == null || spent == null) return null;
    return { at: num(r.at) ?? 0, scope: r.scope === "route" ? "route" : "agent", cap, spent, route: str(r.route), place: str(r.place) };
}

/** Whether a card was written on an earlier UTC day than `now`'s: the gateway's day has turned since, so it no longer holds. */
const fromEarlierDay = (card: SpendCard, now: number): boolean => card.at > 0 && Math.floor(card.at / DAY_MS) < Math.floor(now / DAY_MS);

/** The pane's stop card, or null (absent, not the shape srv writes, from an earlier UTC day, or dismissed for this stop). */
export function readSpendStopped(meta: Record<string, unknown> | null | undefined, now: number = Date.now()): SpendCard | null {
    const card = readCard(meta?.[SPEND_STOPPED_META_KEY]);
    if (!card || fromEarlierDay(card, now)) return null;
    return num(meta?.[SPEND_DISMISSED_META_KEY]) === card.at ? null : card;
}

/** The 80% warning, or null (absent, from an earlier UTC day, or the stop shows instead). */
export function readSpendWarning(meta: Record<string, unknown> | null | undefined, now: number = Date.now()): SpendCard | null {
    const stop = readCard(meta?.[SPEND_STOPPED_META_KEY]);
    if (stop && !fromEarlierDay(stop, now)) return null;
    const card = readCard(meta?.[SPEND_WARNING_META_KEY]);
    return card && !fromEarlierDay(card, now) ? card : null;
}

/** The UTC day number, read again just after each UTC midnight, so a card read against it expires without another event. */
export function createUtcDay(): Accessor<number> {
    const today = () => Math.floor(Date.now() / DAY_MS);
    const [day, setDay] = createSignal(today());
    let timer: ReturnType<typeof setTimeout> | undefined;
    const arm = () => {
        timer = setTimeout(() => {
            setDay(today());
            arm();
        }, DAY_MS - (Date.now() % DAY_MS) + 1000);
    };
    arm();
    onCleanup(() => clearTimeout(timer));
    return day;
}

/** The pane's stop card and warning, each gone once its UTC day is over without waiting for another event from srv. */
export function createSpendCards(meta: Accessor<Record<string, unknown> | null | undefined>) {
    const day = createUtcDay();
    return {
        stopped: () => {
            day();
            return readSpendStopped(meta());
        },
        warning: () => {
            day();
            return readSpendWarning(meta());
        },
    };
}

/** Today's figure for the chips, or null when none was written or it is from an earlier UTC day. */
export function readSpendToday(meta: Record<string, unknown> | null | undefined, now: number = Date.now()): SpendToday | null {
    const raw = meta?.[SPEND_TODAY_META_KEY];
    if (raw == null || typeof raw !== "object") return null;
    const r = raw as Record<string, unknown>;
    const spent = num(r.spent);
    if (spent == null || num(r.day) !== Math.floor(now / DAY_MS)) return null;
    return { spent, cap: num(r.cap) };
}

const usd = (n: number): string => (Number.isInteger(n) ? `$${n}` : `$${n.toFixed(2)}`);

/** "$4.20 of $20 today", or "$4.20 today" for an agent with no cap. */
export function spendChipText(today: SpendToday): string {
    return today.cap == null ? `${usd(today.spent)} today` : `${usd(today.spent)} of ${usd(today.cap)} today`;
}

/** The chip's tone: warn from 80% of the cap, error at the cap. */
export function spendChipTone(today: SpendToday): "neutral" | "warn" | "error" {
    if (today.cap == null || today.cap <= 0) return "neutral";
    const used = today.spent / today.cap;
    return used >= 1 ? "error" : used >= 0.8 ? "warn" : "neutral";
}

/** What the chip's tooltip says. `outOfCredits`: the route's balance stopped the agent, not its cap (#47477). */
export function spendChipTitle(today: SpendToday, outOfCredits = false): string {
    if (outOfCredits) {
        return `What this agent has used through its model route today (UTC). It is stopped because the route has no credits left, not by a cap: add credits to the route's key in the provider's account.`;
    }
    if (today.cap == null) return "What this agent has used through its model route today (UTC). It has no daily cap.";
    return `What this agent has used through its model route today (UTC), against its daily cap of ${usd(today.cap)}. The gateway stops its requests at the cap; its leader or you can raise it.`;
}

/** The card's title: "Stopped: this agent used $20 of its $20 today." */
export function spendStoppedText(card: SpendCard): string {
    return card.scope === "route"
        ? `Stopped: the cap for ${card.place || "this model route"} is used today (${usd(card.spent)} of ${usd(card.cap)}).`
        : `Stopped: this agent used ${usd(card.spent)} of its ${usd(card.cap)} today.`;
}

/** The card's second line. */
export function spendStoppedHint(card: SpendCard): string {
    return card.scope === "route"
        ? "Every agent on the route is stopped until the route's cap is raised in Settings or the UTC day ends."
        : "Its requests are refused until you raise the cap, or the UTC day ends.";
}

/** The 80% line. */
export function spendWarningText(card: SpendCard): string {
    return `${usd(card.spent)} of ${usd(card.cap)} used today: the agent stops at its cap.`;
}

/** What Raise offers: double the cap, at least $5 more. */
export function raisedCap(cap: number): number {
    return Math.ceil(Math.max(cap * 2, cap + 5));
}

/** Sets the agent's daily cap (null: no cap). Rejects with srv's reason. */
export async function setSpendCap(blockId: string, cap: number | null): Promise<void> {
    await RpcApi.AgentSpendCommand(TabRpcClient, { blockid: blockId, action: "set", cap_usd: cap });
}

/** Zeroes today's figure. Rejects with srv's reason. */
export async function resetSpendToday(blockId: string): Promise<void> {
    await RpcApi.AgentSpendCommand(TabRpcClient, { blockid: blockId, action: "reset", cap_usd: null });
}

/** Takes the card off the pane; the stop stays. Best effort. */
export async function dismissSpendStopped(blockId: string, card: SpendCard): Promise<void> {
    try {
        await RpcApi.SetMetaCommand(TabRpcClient, { oref: MOS.makeORef("block", blockId), meta: { [SPEND_DISMISSED_META_KEY]: card.at } });
    } catch {
        // Best effort.
    }
}
