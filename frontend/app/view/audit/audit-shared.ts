// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the Audit pane asks `audit.query` for, and how it words the answer. The query itself, its paging and
// its filters are srv's (crates/srv/src/backend/audit_query.rs).

import type { AuditCost } from "@/types/rpc/AuditCost";
import type { AuditEnd } from "@/types/rpc/AuditEnd";
import type { AuditEntry } from "@/types/rpc/AuditEntry";
import type { AuditHistoryLink } from "@/types/rpc/AuditHistoryLink";
import type { AuditQueryReq } from "@/types/rpc/AuditQueryReq";
import type { AuditTotals } from "@/types/rpc/AuditTotals";

export const AUDIT_PAGE = 50;

/** Pane meta naming the agent the Audit is filtered to (the Swarm's "Audit for this agent" sets it). */
export const AUDIT_AGENT_META_KEY = "audit:agent";

/** The kinds the query knows, in the order the chips show them. */
export const KIND_CHIPS: { kind: string; label: string }[] = [
    { kind: "jekt", label: "Jekts" },
    { kind: "action", label: "Actions" },
    { kind: "registry", label: "Registrations" },
    { kind: "agent_start", label: "Starts" },
    { kind: "agent_end", label: "Ends" },
    { kind: "tool", label: "Tool calls" },
    { kind: "thought", label: "Thoughts" },
];

/** What the list shows until a chip is changed: everything but the bulk (tool calls and thoughts). */
export const DEFAULT_KINDS: readonly string[] = ["jekt", "action", "registry", "agent_start", "agent_end"];

export type AuditRange = "all" | "1h" | "24h" | "7d";

export const RANGES: { value: AuditRange; label: string; ms: number }[] = [
    { value: "all", label: "All time", ms: 0 },
    { value: "1h", label: "Last hour", ms: 60 * 60 * 1000 },
    { value: "24h", label: "Last 24 hours", ms: 24 * 60 * 60 * 1000 },
    { value: "7d", label: "Last 7 days", ms: 7 * 24 * 60 * 60 * 1000 },
];

export interface AuditFilter {
    agent: string;
    kinds: ReadonlySet<string>;
    range: AuditRange;
    text: string;
}

export const emptyFilter = (agent = ""): AuditFilter => ({ agent, kinds: new Set(DEFAULT_KINDS), range: "all", text: "" });

/** The kinds as the query takes them: left out when they are the defaults, which are srv's too. */
export function requestKinds(kinds: ReadonlySet<string>): string[] | undefined {
    const same = kinds.size === DEFAULT_KINDS.length && DEFAULT_KINDS.every((k) => kinds.has(k));
    return kinds.size === 0 || same ? undefined : KIND_CHIPS.map((c) => c.kind).filter((k) => kinds.has(k));
}

export function buildRequest(filter: AuditFilter, now: number, cursor?: string, totals = false): AuditQueryReq {
    const span = RANGES.find((r) => r.value === filter.range)?.ms ?? 0;
    const agent = filter.agent.trim();
    const text = filter.text.trim();
    return {
        limit: AUDIT_PAGE,
        ...(requestKinds(filter.kinds) ? { kinds: requestKinds(filter.kinds) } : {}),
        ...(agent ? { agent_id: agent } : {}),
        ...(text ? { text } : {}),
        ...(span ? { since_ms: now - span } : {}),
        ...(cursor ? { cursor } : {}),
        ...(totals ? { totals: true } : {}),
    };
}

/** A row's failure shows in its colour; srv words a failed jekt or action as "failed" in the summary. */
export const isFailure = (entry: AuditEntry): boolean => entry.kind !== "registry" && /\bfailed\b/.test(entry.summary);

export function formatTokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 10_000) return `${Math.round(n / 1000)}k`;
    if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
    return String(n);
}

/** "~120 tokens" for an estimate, "120 tokens" for what a provider reported. */
export function costLabel(cost: AuditCost): string {
    const tokens = cost.input + cost.cache_read + cost.output;
    const approx = cost.basis === "estimated" ? "~" : "";
    const usd = cost.usd !== undefined ? ` · ${approx}$${cost.usd.toFixed(cost.usd < 0.01 ? 4 : 2)}` : "";
    return `${approx}${formatTokens(tokens)} tokens${usd}`;
}

export function totalsLabel(totals: AuditTotals): string {
    const approx = totals.estimated ? "~" : "";
    const tokens = totals.input + totals.cache_read + totals.output;
    const usd = totals.usd !== undefined ? ` · ${approx}$${totals.usd.toFixed(2)}` : "";
    const noun = totals.entries === 1 ? "entry" : "entries";
    return `${totals.entries} ${noun} · ${approx}${formatTokens(tokens)} tokens${usd}`;
}

/** One button on a row: opens an agent's history at the entry, or reveals its live pane. */
export type AuditLinkItem =
    | { key: string; label: string; kind: "history"; link: AuditHistoryLink }
    | { key: string; label: string; kind: "pane"; blockId: string };

function linksOf(end: Pick<AuditEnd, "links">, key: string, who: string): AuditLinkItem[] {
    const items: AuditLinkItem[] = [];
    if (end.links.history) {
        items.push({ key: `${key}-history`, label: who ? `${who}'s history` : "In history", kind: "history", link: end.links.history });
    }
    if (end.links.pane) {
        items.push({ key: `${key}-pane`, label: who ? `${who}'s pane` : "In its pane", kind: "pane", blockId: end.links.pane.block_id });
    }
    return items;
}

/**
 * The buttons a row has, only those that can work. A jekt names both ends; an action has its actor's links and its
 * target's; every other kind has its own agent's.
 */
export function entryLinks(entry: AuditEntry): AuditLinkItem[] {
    if (entry.kind === "jekt") {
        return [...(entry.from ? linksOf(entry.from, "from", "Sender") : []), ...(entry.to ? linksOf(entry.to, "to", "Receiver") : [])];
    }
    if (entry.kind === "action") {
        return [...linksOf(entry, "actor", "Actor"), ...(entry.to ? linksOf(entry.to, "target", "Target") : [])];
    }
    return linksOf(entry, "own", "");
}

/** What the "not recorded" answer says, for the kinds the query has no entries for yet. */
export function notRecordedNote(kinds: string[]): string {
    const names = kinds.map((k) => KIND_CHIPS.find((c) => c.kind === k)?.label.toLowerCase() ?? k);
    return `Not recorded yet: ${names.join(" and ")}.`;
}
