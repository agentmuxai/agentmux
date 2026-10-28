// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's Broadcast section: the composer docked under the shared agent list, and the last ten
// broadcasts sent from this Swarm. The draft and the recent list live outside the section, so
// switching away and back loses neither. docs/specs/SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §3.3.

import { createMemo, createSignal, For, onCleanup, Show, type Accessor, type JSX, type Setter } from "solid-js";
import { Button, TextArea } from "@/app/element/ui";
import { keyLabel } from "@/app/keybindings";
import type { SwarmAgents } from "./swarm-agent-list";
import { gapSummary, localFleetTargets, partitionTargets, remoteFleetTargets } from "./swarm-fleet-targets";
import type { SwarmViewModel } from "./swarm-model";
import { remoteSections } from "./swarm-remote";

/** One broadcast sent from this Swarm, kept in memory only. */
export interface BroadcastRecord {
    text: string;
    at: number;
    recipients: number;
    delivered: number;
    failed: number;
}

export const MAX_RECENT_BROADCASTS = 10;

/** `list` with `record` newest first, capped at the last ten. */
export function withRecent(list: BroadcastRecord[], record: BroadcastRecord): BroadcastRecord[] {
    return [record, ...list].slice(0, MAX_RECENT_BROADCASTS);
}

interface BroadcastState {
    draft: Accessor<string>;
    setDraft: Setter<string>;
    recent: Accessor<BroadcastRecord[]>;
    setRecent: Setter<BroadcastRecord[]>;
}

const states = new WeakMap<object, BroadcastState>();

/** The draft and recent broadcasts of one Swarm pane, created on first use and kept for its life. */
function broadcastStateOf(model: object): BroadcastState {
    let state = states.get(model);
    if (!state) {
        const [draft, setDraft] = createSignal("");
        const [recent, setRecent] = createSignal<BroadcastRecord[]>([]);
        state = { draft, setDraft, recent, setRecent };
        states.set(model, state);
    }
    return state;
}

function ago(at: number, now: number): string {
    const s = Math.max(0, Math.round((now - at) / 1000));
    if (s < 60) return "just now";
    if (s < 3600) return `${Math.floor(s / 60)} min ago`;
    return `${Math.floor(s / 3600)} h ago`;
}

export function BroadcastDock(props: { model: SwarmViewModel; agents: Pick<SwarmAgents, "tree"> }): JSX.Element {
    const model = props.model;
    const state = broadcastStateOf(model);
    // From the list the pane already keeps (which rebuilds only while the pane is visible), not
    // `model.fleetTargets()`, which would rebuild the agent tree on every agent event.
    const known = createMemo(
        () => new Map([...localFleetTargets(props.agents.tree()), ...remoteFleetTargets(remoteSections(model.otherInstancesAtom()))])
    );
    const split = createMemo(() => partitionTargets([...model.selectedBlockIdsAtom()], "broadcast", known()));
    const recipients = () => split().reachable.length;
    const selected = () => model.selectedBlockIdsAtom().size;
    const gap = () => gapSummary(split().unreachable, selected(), "broadcast");
    const canSend = () => state.draft().trim().length > 0 && recipients() > 0 && !model.fleetActionInFlightAtom();
    // The ages in the recent list tick; the list itself changes only on a send.
    const [now, setNow] = createSignal(Date.now());
    const ticker = setInterval(() => setNow(Date.now()), 30_000);
    onCleanup(() => clearInterval(ticker));

    const send = async (): Promise<void> => {
        if (!canSend()) return;
        const text = state.draft().trim();
        const count = recipients();
        // The result also lists the targets that could not be reached; they are not among `count`.
        const unreachable = split().unreachable.length;
        state.setDraft("");
        let result;
        try {
            result = await model.broadcastToSelection(text);
        } catch {
            // The call itself failed: nothing was sent, so the message is not lost.
            if (state.draft() === "") state.setDraft(text);
            return;
        }
        state.setRecent((list) =>
            withRecent(list, {
                text,
                at: Date.now(),
                recipients: count,
                delivered: result.succeeded.length,
                failed: Math.max(0, result.failed.length - unreachable),
            })
        );
    };

    return (
        <div class="swarm-broadcast-dock">
            <TextArea
                class="swarm-broadcast-input"
                rows={3}
                placeholder={
                    selected() === 0 ? "Select agents to message…" : `Message to send to ${recipients()} agent${recipients() === 1 ? "" : "s"}…`
                }
                value={state.draft()}
                onInput={(e) => state.setDraft(e.currentTarget.value)}
                onKeyDown={(e) => {
                    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                        e.preventDefault();
                        void send();
                    }
                }}
            />
            <div class="swarm-broadcast-actions">
                <Button tone="accent" class="swarm-fleet-btn swarm-fleet-btn--primary" disabled={!canSend()} onClick={() => void send()}>
                    <i class="fa-solid fa-tower-broadcast" /> Send to {recipients()}
                </Button>
                <span class="swarm-broadcast-hint">{keyLabel("mod+Enter")} sends</span>
                <Show when={gap()}>{(text) => <span class="swarm-fleet-broadcast-gap">{text()}</span>}</Show>
            </div>
            <Show when={state.recent().length > 0}>
                <div class="swarm-broadcast-recent" aria-label="Recent broadcasts">
                    <For each={state.recent()}>
                        {(r) => (
                            <div class="swarm-broadcast-recent-row" title={r.text}>
                                <span class="swarm-broadcast-recent-text">{r.text}</span>
                                <span class="swarm-broadcast-recent-meta">
                                    {ago(r.at, now())} · {r.recipients} sent · {r.delivered} delivered
                                    <Show when={r.failed > 0}> · {r.failed} failed</Show>
                                </span>
                            </div>
                        )}
                    </For>
                </div>
            </Show>
        </div>
    );
}
