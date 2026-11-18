// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What waits on the person, as the parts of one summary (docs/specs/SPEC_STATUS_BAR_COMPACT_SUMMARY_AND_VAULT_REQUESTS_2026_11_15.md §2):
 * agents running, Asks (questions, tool permissions, consents, confirms, installs), Browser, Vault, agents signed out or
 * out of quota, agents with messages held for them, and Kanban tasks.
 * A pure function over what the window already knows, so each rule is a test and not a screenshot.
 *
 * Everything is counted once. A pane that waits only on a Vault request is the Vault's row, not also an ask; the Vault
 * part is every request the Vault itself holds (`uses` and `requests`), whatever pane asked, so a request whose pane
 * closed is still found. A request that belongs to no pane has a row: it plays the tone, so it has somewhere to be found.
 */

import type { AttentionItem } from "@/app/notification/attention-list";
import type { EndedAsk } from "@/app/notification/ended-asks";
import type { WaitingApp } from "@/app/notification/waiting-for-you";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { JektHeldAgent } from "@/types/rpc/JektHeldAgent";
import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import { resetLabel } from "@/util/reset-label";
import { KIND_PLACES, type PartId } from "./summary-kinds";

/** Where a row's button goes. */
export type SummaryClick =
    | { to: "pane"; item: AttentionItem }
    | { to: "app"; kind: string; key: string }
    | { to: "vault"; id: string }
    | { to: "swarm" }
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
    /** How many things wait on the person (Asks, Browser, Vault, signed-out and out-of-quota agents, and tasks): the badge when the bar is narrow. */
    waiting: number;
    /** The asks that already ended, newest first (a list of their own: they are not waiting, so not in `waiting`). */
    ended: EndedAsk[];
}

export interface SummaryInput {
    /** Agents that have a process. */
    running: number;
    /** Panes that wait, from this window's registry joined with srv's agent rows. */
    items: AttentionItem[];
    /** Requests that belong to no pane. */
    apps: WaitingApp[];
    /** srv's agent rows: the ones signed out or out of quota are parts of their own. */
    agents?: AgentRuntime[];
    /** Messages srv holds for agents, as counts; the ones held for an agent that is not running are a part. */
    held?: JektHeldAgent[];
    uses: UseRequest[];
    requests: VaultRequest[];
    /** Kanban tasks that need the person. */
    tasks: number;
    /** The asks that ended in the last two minutes. */
    ended?: EndedAsk[];
}

/** Whether a pane waits only on Vault requests: the Vault's own lists count those, so it is not also an agent waiting. */
export function isVaultOnly(item: Pick<AttentionItem, "kinds">): boolean {
    const kinds = item.kinds ?? [];
    return kinds.length > 0 && kinds.every((k) => KIND_PLACES[k]?.part === "vault");
}

/** The part of a pane that waits: all Vault, and the Vault's lists have it; all Browser; or an ask. */
function paneRows(items: AttentionItem[]): SummaryRow[] {
    const rows: SummaryRow[] = [];
    for (const item of items) {
        if (isVaultOnly(item)) continue;
        const parts = (item.kinds ?? []).map((k) => KIND_PLACES[k]?.part);
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

/** What an agent that is held by its account wants of the person: a sign-in, or a wait for its quota to come back. */
const SIGNED_OUT_WHAT = "needs you to sign in again";
const limitedWhat = (resetsAt?: number) => (resetsAt ? `is out of quota until ${resetLabel(resetsAt)}` : "is out of quota");

/** The agents that srv says are signed out or out of quota. A pane that already has a row as an ask is not listed twice. */
function accountRows(agents: AgentRuntime[], items: AttentionItem[]): SummaryRow[] {
    const asked = new Set(items.map((i) => i.blockId));
    const rows: SummaryRow[] = [];
    for (const a of agents) {
        if (a.state !== "signed_out" && a.state !== "limited") continue;
        if (a.block_id && asked.has(a.block_id)) continue;
        // The pane where it is answered when the agent has one; otherwise the Swarm, where every agent is found.
        const click: SummaryClick = a.block_id
            ? { to: "pane", item: { blockId: a.block_id, name: a.name, agentId: a.agent_id, color: a.color, what: "", since: 0, local: true } }
            : { to: "swarm" };
        rows.push({
            key: `account:${a.agent_id}`,
            part: a.state === "signed_out" ? "signedout" : "quota",
            name: a.name || "An agent",
            color: a.color,
            what: a.state === "signed_out" ? SIGNED_OUT_WHAT : limitedWhat(a.resets_at),
            // How long it has been held is not known here, so the row shows no age.
            since: 0,
            click,
        });
    }
    return rows;
}

/** The agents srv holds messages for because they are not running and could not be started: the person starts them. The other
 *  reasons mend by themselves or are the signed-out and quota parts' own. */
function heldRows(held: JektHeldAgent[]): SummaryRow[] {
    return held
        .filter((h) => h.reason === "not_running")
        .map((h) => ({
            key: `held:${h.agent_id}`,
            part: "held" as const,
            name: h.agent_name || "An agent",
            what: h.count === 1 ? "has 1 message waiting: it is not running" : `has ${h.count} messages waiting: it is not running`,
            since: h.since_ms,
            click: { to: "swarm" } as SummaryClick,
        }));
}

export function summarize(input: SummaryInput): Summary {
    const rows = [
        ...paneRows(input.items),
        ...appRows(input.apps),
        ...vaultRows(input.uses, input.requests, input.items),
        ...accountRows(input.agents ?? [], input.items),
        ...heldRows(input.held ?? []),
    ];
    // Oldest wait first; the agents held by an account have no age, so they keep srv's order (a stable sort).
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
        { id: "signedout", count: byPart("signedout").length, rows: byPart("signedout") },
        { id: "quota", count: byPart("quota").length, rows: byPart("quota") },
        { id: "held", count: byPart("held").length, rows: byPart("held") },
        // A task row stands for all of them; the count is the Kanban's own.
        { id: "tasks", count: input.tasks, rows: taskRows },
    ];
    return { parts, waiting: parts.filter((p) => p.id !== "agents").reduce((n, p) => n + p.count, 0), ended: input.ended ?? [] };
}
