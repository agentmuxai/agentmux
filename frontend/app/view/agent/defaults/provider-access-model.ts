// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Free and Local tags and the card filters, as data: what each tag says,
 * in which tone, and how fresh the claims behind it are. Pure functions of
 * `provider-access.ts` and a date, so a stale claim is a computed state, not a
 * timer. The card renders these with the kit's `Tag`
 * (docs/specs/SPEC_PROVIDER_CARD_TAGS_FREE_LOCAL_2026_10_29.md §3 and §5).
 */

import {
    getProviderAccess,
    type AccessClaim,
    type FreeAccess,
    type LocalAccess,
    type ProviderAccess,
} from "./provider-access";

/** A claim this old gets "checked N days ago, may have changed". */
export const AGING_DAYS = 30;
/** A claim this old turns the Free tag neutral: "Not re-checked since ...". */
export const STALE_DAYS = 90;

export type Freshness = "fresh" | "aging" | "stale";

export interface CheckedNote {
    /** The oldest `verifiedOn` among the claims the tag rests on. */
    date: string;
    state: Freshness;
    /** The muted line at the end of the tooltip. */
    text: string;
}

export interface AccessTag {
    label: "Free" | "Local";
    /** `accent` is blue in the default theme; a stale Free tag is `neutral`. */
    tone: "accent" | "neutral";
    icon: string;
    /** Tooltip lines, at most four, in order. */
    lines: string[];
    checked: CheckedNote;
}

const DAY_MS = 24 * 60 * 60 * 1000;

function dayNumber(iso: string): number {
    const [y, m, d] = iso.split("-").map(Number);
    return Date.UTC(y, m - 1, d) / DAY_MS;
}

/** Whole days from `iso` to `now` (UTC dates); negative for a date in the future. */
export function daysSince(iso: string, now: Date): number {
    return Math.floor(now.getTime() / DAY_MS) - dayNumber(iso);
}

function oldest(claims: AccessClaim[]): string {
    return claims.map((c) => c.verifiedOn).reduce((a, b) => (dayNumber(a) <= dayNumber(b) ? a : b));
}

export function checkedNote(claims: AccessClaim[], now: Date): CheckedNote {
    const date = oldest(claims);
    const days = daysSince(date, now);
    if (days > STALE_DAYS) return { date, state: "stale", text: `Not re-checked since ${date}` };
    if (days > AGING_DAYS) return { date, state: "aging", text: `checked ${days} days ago, may have changed` };
    return { date, state: "fresh", text: `checked ${date}` };
}

function freeClaims(free: FreeAccess): AccessClaim[] {
    return [free.limits, ...free.catches];
}

/** The Free tag for a provider: none unless it has a free route that has not expired. */
export function freeTagFor(access: ProviderAccess | undefined, now: Date): AccessTag | null {
    const free = access?.free;
    if (!free) return null;
    if (free.expiresOn && daysSince(free.expiresOn, now) > 0) return null;
    const checked = checkedNote(freeClaims(free), now);
    const lines = [free.lead, free.limits.text, ...free.catches.slice(0, 2).map((c) => c.text)];
    return {
        label: "Free",
        tone: checked.state === "stale" ? "neutral" : "accent",
        icon: "gift",
        lines,
        checked,
    };
}

/** The Local tag: the CLI documents a way to use a model on this computer. */
export function localTagFor(access: ProviderAccess | undefined, now: Date): AccessTag | null {
    const local: LocalAccess | undefined = access?.local;
    if (!local) return null;
    const claims = local.note ? [local.how, local.note] : [local.how];
    const lines = [local.how.text];
    if (local.note) lines.push(local.note.text);
    lines.push(
        local.testedOn
            ? `${local.testedOn.ok ? "Works" : "Did not work"} in AgentMux's own test with ${local.testedOn.model} (${local.testedOn.date}).`
            : "Documented by the CLI, not tested by AgentMux."
    );
    return { label: "Local", tone: "neutral", icon: "house", lines, checked: checkedNote(claims, now) };
}

/** The tags a provider's card shows, in the kit's order: Free, then Local. */
export function accessTagsFor(provider: string, now: Date): AccessTag[] {
    const access = getProviderAccess(provider);
    return [freeTagFor(access, now), localTagFor(access, now)].filter((t): t is AccessTag => t !== null);
}

/** The line for the new-agent panel when a provider has a plain answer to give. */
export function plainNoteFor(provider: string, now: Date): { text: string; checked: CheckedNote } | null {
    const plain = getProviderAccess(provider)?.plain;
    return plain ? { text: plain.text, checked: checkedNote([plain], now) } : null;
}

// ── Card filters ────────────────────────────────────────────────────────

export type CardFilter = "free" | "local" | "installed";
export const CARD_FILTERS: readonly CardFilter[] = ["free", "local", "installed"];

export interface FilterableCard {
    provider: string;
    /** From the picker's install probe; `undefined` while it hasn't answered. */
    installed: boolean | undefined;
}

function hasFilter(card: FilterableCard, filter: CardFilter, now: Date): boolean {
    switch (filter) {
        case "free":
            return freeTagFor(getProviderAccess(card.provider), now) !== null;
        case "local":
            return localTagFor(getProviderAccess(card.provider), now) !== null;
        case "installed":
            return card.installed === true;
    }
}

/** Selecting more than one narrows: a card has to pass every selected filter. */
export function matchesFilters(card: FilterableCard, selected: ReadonlySet<CardFilter>, now: Date): boolean {
    for (const f of selected) if (!hasFilter(card, f, now)) return false;
    return true;
}

/**
 * Each chip's count: how many cards would be shown with that chip selected on
 * top of what is selected now. A selected chip's count is the current result.
 */
export function filterCounts(
    cards: readonly FilterableCard[],
    selected: ReadonlySet<CardFilter>,
    now: Date
): Record<CardFilter, number> {
    const counts = { free: 0, local: 0, installed: 0 } as Record<CardFilter, number>;
    for (const f of CARD_FILTERS) {
        const withIt = new Set(selected).add(f);
        counts[f] = cards.filter((c) => matchesFilters(c, withIt, now)).length;
    }
    return counts;
}
