// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What waits on the person, as the parts of one summary (docs/specs/SPEC_STATUS_BAR_COMPACT_SUMMARY_AND_VAULT_REQUESTS_2026_11_15.md §2):
 * agents running, Asks (questions, tool permissions, consents, confirms, installs), Browser, Vault and Kanban tasks.
 * A pure function over what the window already knows, so each rule is a test and not a screenshot.
 *
 * Everything is counted once. A pane that waits only on a Vault request is the Vault's row, not also an ask; the Vault
 * part is every request the Vault itself holds (`uses` and `requests`), whatever pane asked, so a request whose pane
 * closed is still found. A request that belongs to no pane has a row: it plays the tone, so it has somewhere to be found.
 */

import type { AttentionItem } from "@/app/notification/attention-list";
import type { WaitingApp } from "@/app/notification/waiting-for-you";
import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import { KIND_PLACES, type PartId } from "./summary-kinds";

/** Where a row's button goes. */
export type SummaryClick =
    | { to: "pane"; item: AttentionItem }
    | { to: "app"; kind: string; key: string }
    | { to: "vault"; id: string }
    | { to: "kanban" };

export interface SummaryRow {
    /** Unique within the summary. */
    key: string;
    part: Exclude<PartId, "agents">;
    /** Who asks: an agent's name, the pane's name, "Kanban", or "AgentMux" for a request with no pane. */
    name: string;
    color?: string;
    /** What it wants, in a few words, led by a verb. */
    what: string;
    /** When it began to wait (unix ms). */
    since: number;
    /** A Vault use that goes ahead on its own: when (unix ms). */
    goesAheadAtMs?: number;
    click: SummaryClick;
}

export interface SummaryPart {
    id: PartId;
    count: number;
    rows: SummaryRow[];
}

export interface Summary {
    /** Every part, in order, including those with a zero count. */
    parts: SummaryPart[];
    /** How many things wait on the person (Asks, Browser, Vault and tasks): the badge when the bar is narrow. */
    waiting: number;
}

export interface SummaryInput {
    /** Agents that have a process. */
    running: number;
    /** Panes that wait, from this window's registry joined with srv's agent rows. */
    items: AttentionItem[];
    /** Requests that belong to no pane. */
    apps: WaitingApp[];
    uses: UseRequest[];
    requests: VaultRequest[];
    /** Kanban tasks that need the person. */
    tasks: number;
}

/** The part of a pane that waits: all Vault, and the Vault's lists have it; all Browser; or an ask. */
function paneRows(items: AttentionItem[]): SummaryRow[] {
    const rows: SummaryRow[] = [];
    for (const item of items) {
        const parts = (item.kinds ?? []).map((k) => KIND_PLACES[k]?.part);
        if (parts.length > 0 && parts.every((p) => p === "vault")) continue;
        const part = parts.length > 0 && parts.every((p) => p === "browser") ? "browser" : "asks";
        rows.push({ key: `pane:${item.blockId}`, part, name: item.name, color: item.color, what: item.what, since: item.since, click: { to: "pane", item } });
    }
    return rows;
}

function appRows(apps: WaitingApp[]): SummaryRow[] {
    const rows: SummaryRow[] = [];
    for (const a of apps) {
        const part = KIND_PLACES[a.kind]?.part ?? "asks";
        // The Vault's own lists have it.
        if (part === "vault") continue;
        rows.push({ key: `app:${a.key}`, part, name: "AgentMux", what: a.what, since: a.since, click: { to: "app", kind: a.kind, key: a.key } });
    }
    return rows;
}

function vaultRows(uses: UseRequest[], requests: VaultRequest[], items: AttentionItem[]): SummaryRow[] {
    const known = new Map(items.map((i) => [i.blockId, i]));
    // The pane's own card when this window knows the pane; otherwise the Vault's Requests list.
    const click = (blockId: string, id: string): SummaryClick => {
        const item = blockId ? known.get(blockId) : undefined;
        return item ? { to: "pane", item } : { to: "vault", id };
    };
    const rows: SummaryRow[] = uses.map((u) => ({
        key: `vault-use:${u.id}`,
        part: "vault" as const,
        name: u.agent || "An agent",
        what: u.entry_label ? `asks to use ${u.entry_label}` : "asks to use a secret",
        since: u.requested_ms,
        goesAheadAtMs: u.goes_ahead ? u.deadline_ms : undefined,
        click: click(u.block_id, u.id),
    }));
    for (const r of requests) {
        rows.push({
            key: `vault-request:${r.id}`,
            part: "vault",
            name: r.agent || "An agent",
            what: r.label ? `asks to add ${r.label}` : "asks to add a new entry",
            since: r.requested_ms,
            click: click(r.block_id, r.id),
        });
    }
    return rows;
}

export function summarize(input: SummaryInput): Summary {
    const rows = [...paneRows(input.items), ...appRows(input.apps), ...vaultRows(input.uses, input.requests, input.items)];
    const byPart = (part: SummaryRow["part"]) => rows.filter((r) => r.part === part).sort((a, b) => a.since - b.since);
    const taskRows: SummaryRow[] =
        input.tasks > 0
            ? [{ key: "kanban", part: "tasks", name: "Kanban", what: input.tasks === 1 ? "1 task needs you" : `${input.tasks} tasks need you`, since: 0, click: { to: "kanban" } }]
            : [];
    const parts: SummaryPart[] = [
        { id: "agents", count: input.running, rows: [] },
        { id: "asks", count: byPart("asks").length, rows: byPart("asks") },
        { id: "browser", count: byPart("browser").length, rows: byPart("browser") },
        { id: "vault", count: byPart("vault").length, rows: byPart("vault") },
        // A task row stands for all of them; the count is the Kanban's own.
        { id: "tasks", count: input.tasks, rows: taskRows },
    ];
    return { parts, waiting: parts.filter((p) => p.id !== "agents").reduce((n, p) => n + p.count, 0) };
}
