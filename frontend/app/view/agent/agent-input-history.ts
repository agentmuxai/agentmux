// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * What the user typed to an agent, kept with the agent by srv
 * (`agent.input-history*`, `agent.input-draft`; crates/srv/src/backend/storage/
 * agent_input_history.rs, docs/specs/SPEC_AGENT_INPUT_HISTORY_2026_10_18.md),
 * so ArrowUp recalls it and the draft comes back after a restart, an upgrade
 * or in a new pane. A pane without a defined agent (a quick-launch provider
 * pane) keeps the per-pane history of reload-persistence.ts instead.
 *
 * One cache per agent, shared by every pane showing it: `entries` oldest
 * first, the array the composer walks. srv's newest page arrives after the
 * pane mounts, older pages on demand, and sends from another pane or window
 * after the `agent:inputhistory` event; each arrival tells the composers
 * watching (`watchAgentHistory`) where entries went, so their cursors stay on
 * the same entry.
 */

import { WpsEvent } from "@/app/store/mps-events";
import { onMuxEvent } from "@/app/store/mps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { isBangCommand } from "./bang-command";
import { PROVIDERS } from "./providers";

/** Entries read from srv at a time. */
const PAGE = 200;
/** How long the draft waits after the last keystroke before it is kept. */
const DRAFT_WRITE_DELAY_MS = 500;

/** Where new entries went: before the oldest shown, or after the newest. */
export type Arrival = "older" | "newer";

type Cache = {
    /** Oldest first: the array the composer walks. */
    entries: string[];
    /** srv's id for each entry, 0 while a send's id is on its way. */
    ids: number[];
    /** Nothing older left in srv. */
    complete: boolean;
    loading: Promise<void> | null;
    /** Sends whose answer hasn't come back. */
    pending: number;
    watchers: Set<(where: Arrival, n: number) => void>;
};

const caches = new Map<string, Cache>();
let subscribed = false;

/** The agent a pane's input history belongs to: its definition id, not a quick-launch provider key. */
export function historyAgentId(meta: Record<string, unknown> | null | undefined): string | null {
    const id = meta?.["agentId"];
    return typeof id === "string" && id !== "" && !PROVIDERS[id] ? id : null;
}

/** The kind srv records a send as. */
export function sendKind(text: string): "message" | "slash" | "bang" {
    if (isBangCommand(text)) return "bang";
    return text.startsWith("/") ? "slash" : "message";
}

function tell(c: Cache, where: Arrival, n: number): void {
    if (n > 0) for (const w of c.watchers) w(where, n);
}

/** Put `newestFirst` (srv's order) before what is shown; returns how many went in. */
function prepend(c: Cache, newestFirst: { id: number; text: string }[]): number {
    const known = new Set(c.ids);
    const fresh = newestFirst.filter((e) => !known.has(e.id)).reverse();
    c.entries.unshift(...fresh.map((e) => e.text));
    c.ids.unshift(...fresh.map((e) => e.id));
    return fresh.length;
}

function load(agentId: string, c: Cache, beforeId?: number): Promise<void> {
    c.loading ??= RpcApi.AgentInputHistoryCommand(TabRpcClient, { agent_id: agentId, before_id: beforeId, limit: PAGE })
        .then((res) => {
            c.complete = res.entries.length < PAGE;
            tell(c, "older", prepend(c, res.entries));
        })
        .catch((e) => console.warn("[input-history] couldn't read", agentId, e))
        .finally(() => {
            c.loading = null;
        });
    return c.loading;
}

/** Another pane or window sent to an agent shown here: take what is newer than the newest shown. */
function onChanged(agentId: string): void {
    const c = caches.get(agentId);
    // A send of our own answers for itself; a load in flight brings it anyway.
    if (!c || c.pending > 0 || c.loading) return;
    const newest = Math.max(0, ...c.ids);
    void RpcApi.AgentInputHistoryCommand(TabRpcClient, { agent_id: agentId, limit: PAGE })
        .then((res) => {
            const fresh = res.entries.filter((e) => e.id > newest && !c.ids.includes(e.id)).reverse();
            c.entries.push(...fresh.map((e) => e.text));
            c.ids.push(...fresh.map((e) => e.id));
            tell(c, "newer", fresh.length);
        })
        .catch(() => {});
}

function cacheFor(agentId: string): Cache {
    let c = caches.get(agentId);
    if (!c) {
        c = { entries: [], ids: [], complete: false, loading: null, pending: 0, watchers: new Set() };
        caches.set(agentId, c);
        void load(agentId, c);
    }
    if (!subscribed) {
        subscribed = true;
        onMuxEvent(WpsEvent.AgentInputHistoryChanged, (data) => {
            if (data.what === "history") onChanged(data.agent_id);
        });
    }
    return c;
}

/** The agent's entries, oldest first: the same array every call, filled as srv answers. */
export function agentHistory(agentId: string): string[] {
    return cacheFor(agentId).entries;
}

/** Hear where entries arrive in the agent's history; returns the stop. */
export function watchAgentHistory(agentId: string, watcher: (where: Arrival, n: number) => void): () => void {
    const c = cacheFor(agentId);
    c.watchers.add(watcher);
    return () => c.watchers.delete(watcher);
}

/** Read the page before the oldest entry shown, unless all are shown or one is on its way. */
export function loadOlderAgentHistory(agentId: string): Promise<void> {
    const c = cacheFor(agentId);
    if (c.complete || c.loading) return c.loading ?? Promise.resolve();
    const oldest = c.ids.find((id) => id > 0);
    return load(agentId, c, oldest);
}

/**
 * The user sent `text` to the agent: it goes at the end now (a repeat of the
 * newest is not added again), and srv keeps it. A send never waits on this.
 */
export function rememberSent(agentId: string, text: string): void {
    if (!text.trim()) return;
    const c = cacheFor(agentId);
    let at = c.entries.length - 1;
    if (c.entries[at] !== text) {
        c.entries.push(text);
        c.ids.push(0);
        at = c.entries.length - 1;
        // Every composer of this agent in this window shares the array.
        tell(c, "newer", 1);
    }
    c.pending++;
    void RpcApi.AgentInputHistoryAppendCommand(TabRpcClient, { agent_id: agentId, text, kind: sendKind(text) })
        .then((res) => {
            if (res.entry && c.entries[at] === text && c.ids[at] === 0) c.ids[at] = res.entry.id;
        })
        .catch((e) => console.warn("[input-history] couldn't keep a send", agentId, e))
        .finally(() => {
            c.pending--;
        });
}

/** The agent's draft as srv keeps it. */
export async function agentDraft(agentId: string): Promise<string | undefined> {
    try {
        return (await RpcApi.AgentInputDraftCommand(TabRpcClient, { agent_id: agentId })).text ?? undefined;
    } catch {
        return undefined;
    }
}

const draftTimers = new Map<string, ReturnType<typeof setTimeout>>();
const draftPending = new Map<string, string>();

function flushDraft(agentId: string): void {
    clearTimeout(draftTimers.get(agentId));
    draftTimers.delete(agentId);
    const text = draftPending.get(agentId);
    draftPending.delete(agentId);
    if (text === undefined) return;
    void RpcApi.AgentInputDraftCommand(TabRpcClient, { agent_id: agentId, text }).catch(() => {});
}

/** Keep the agent's draft in srv, a moment after the last change; an empty one removes it. */
export function saveAgentDraft(agentId: string, text: string): void {
    draftPending.set(agentId, text);
    clearTimeout(draftTimers.get(agentId));
    draftTimers.set(agentId, setTimeout(() => flushDraft(agentId), DRAFT_WRITE_DELAY_MS));
}

if (typeof window !== "undefined") {
    window.addEventListener("pagehide", () => {
        for (const id of [...draftPending.keys()]) flushDraft(id);
    });
}

/** Test hook: forget every cache, as a new page does. */
export function resetAgentInputHistoryForTests(): void {
    caches.clear();
    subscribed = false;
    for (const t of draftTimers.values()) clearTimeout(t);
    draftTimers.clear();
    draftPending.clear();
}
