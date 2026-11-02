// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The open account episodes: agents out of quota or signed out, grouped by the account they ran on
// (docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6). One shared copy, fetched while any strip or
// badge is mounted: on each `accountavailability` event and every 30 s (a reset passing ends an episode
// with an event, but a missed one must not leave a strip up).

import { createSignal, onCleanup, type Accessor } from "solid-js";

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { resetLabel } from "@/util/reset-label";
import type { AccountAvailability } from "@/types/rpc/AccountAvailability";
import type { IdentityAccountMoveAgentsResult } from "@/types/rpc/IdentityAccountMoveAgentsResult";

const POLL_MS = 30_000;
const DEBOUNCE_MS = 300;

const [episodes, setEpisodes] = createSignal<AccountAvailability[]>([]);
let users = 0;
let unsub: (() => void) | null = null;
let poll: ReturnType<typeof setInterval> | null = null;
let debounce: ReturnType<typeof setTimeout> | null = null;

async function refresh(): Promise<void> {
    try {
        const r = await RpcApi.IdentityAccountAvailabilityCommand(TabRpcClient);
        setEpisodes(r?.episodes ?? []);
    } catch {
        // An older srv, or none yet: no strip.
    }
}

function start(): void {
    void refresh();
    unsub = onMuxEvent(WpsEvent.AccountAvailability, () => {
        if (debounce) clearTimeout(debounce);
        debounce = setTimeout(() => void refresh(), DEBOUNCE_MS);
    });
    poll = setInterval(() => void refresh(), POLL_MS);
}

function stop(): void {
    unsub?.();
    unsub = null;
    if (poll) clearInterval(poll);
    if (debounce) clearTimeout(debounce);
    poll = debounce = null;
}

/** The open episodes, kept current while the calling component is mounted. */
export function useAccountEpisodes(): Accessor<AccountAvailability[]> {
    if (users++ === 0) start();
    onCleanup(() => {
        if (--users === 0) stop();
    });
    return episodes;
}

export { resetLabel };

/** "out of quota until Sun 7:00 PM", "out of quota", or "needs you to sign in again". */
export function episodeReason(ep: Pick<AccountAvailability, "state" | "resets_at_ms">): string {
    if (ep.state === "signed_out") return "needs you to sign in again";
    return ep.resets_at_ms != null ? `out of quota until ${resetLabel(ep.resets_at_ms)}` : "out of quota";
}

/** "Account Work: out of quota until Sun 7:00 PM · Naki-2, Loap-3 · holding #163, #140" (spec §6.2). */
export function episodeLine(ep: AccountAvailability): string {
    const parts = [`Account ${ep.account_label}: ${episodeReason(ep)}`, ep.agents.map((a) => a.agent_name).join(", ")];
    if (ep.tasks.length > 0) parts.push(`holding ${ep.tasks.map((t) => `#${t.number}`).join(", ")}`);
    // A login whose agents already moved has none to name.
    return parts.filter((p) => p !== "").join(" · ");
}

/** The candidate to preselect: the first signed-in one, else the first (spec §13, decision 2). */
export function preselectedCandidate(ep: AccountAvailability): string {
    const signedIn = ep.candidates.find((c) => c.status === "valid" || c.status === "ok");
    return (signedIn ?? ep.candidates[0])?.account_id ?? "";
}

/** What a move did, for the strip to say. */
export function moveOutcome(r: IdentityAccountMoveAgentsResult): string {
    const names = (list: { agent_name: string }[]) => list.map((a) => a.agent_name).join(", ");
    const parts: string[] = [];
    if (r.moved.length > 0) parts.push(`Moved ${names(r.moved)}.`);
    if (r.busy.length > 0) parts.push(`${names(r.busy)}: in a turn; ${r.busy.length === 1 ? "moves" : "move"} when the turn ends.`);
    for (const f of r.failed) parts.push(`${f.agent_name || "A pane"}: ${f.error}`);
    return parts.join(" ") || "Nothing to move.";
}

/** Move the episode's agents (srv moves the idle ones now and queues the rest), then refresh the strip. */
export async function moveEpisodeAgents(ep: AccountAvailability, to: string): Promise<IdentityAccountMoveAgentsResult> {
    const r = await RpcApi.IdentityAccountMoveAgentsCommand(TabRpcClient, {
        from: ep.account_id,
        to,
        block_ids: ep.agents.map((a) => a.block_id),
    });
    void refresh();
    return r;
}
