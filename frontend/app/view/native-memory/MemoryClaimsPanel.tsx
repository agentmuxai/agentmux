// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryClaimsPanel — "Memory folders this agent claims", collapsed, above
 * an agent's file grid in Armory → Memory → Personal
 * (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2, the human "release
 * this folder" action).
 *
 * A claim keeps a folder away from every other agent. It is released on
 * its own when the agent is deleted, not when it moves on — this is how a
 * human releases a stale one. Like adoption, "Release…" only opens the
 * host's confirmation window; it can't release by itself.
 */

import { createEffect, createSignal, For, on, onCleanup, Show, type JSX } from "solid-js";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import { RpcApi, type NativeMemoryClaimList, type NativeMemoryClaimedFolder } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getApi } from "@/store/global";

interface MemoryClaimsPanelProps {
    agentId: string;
    agentName: string;
}

export type ReleaseOutcome = { status: "done" | "declined" | "failed"; text: string };

export function releaseOutcome(payload: any): ReleaseOutcome | null {
    switch (payload?.status) {
        case "done":
            return {
                status: "done",
                text:
                    payload?.report?.released === false
                        ? "That claim was already gone."
                        : "Released. If the agent still uses that folder, its next launch claims it again.",
            };
        case "declined":
            return { status: "declined", text: "Release cancelled." };
        case "failed":
            return { status: "failed", text: `Release failed: ${payload?.error ?? "unknown error"}` };
        default:
            return null;
    }
}

export function claimedSince(ms: number, now = Date.now()): string {
    const days = Math.floor((now - ms) / 86_400_000);
    if (days <= 0) return "today";
    if (days === 1) return "yesterday";
    return `${days} days ago`;
}

export const MemoryClaimsPanel = (props: MemoryClaimsPanelProps): JSX.Element => {
    const [list, setList] = createSignal<NativeMemoryClaimList | null>(null);
    const [open, setOpen] = createSignal(false);
    const [waiting, setWaiting] = createSignal<number | null>(null);
    const [outcome, setOutcome] = createSignal<ReleaseOutcome | null>(null);

    const load = (agentId: string) => {
        RpcApi.NativeMemoryClaimsCommand(TabRpcClient, { agent_id: agentId })
            .then((r) => {
                if (agentId === props.agentId) setList(r);
            })
            .catch(() => setList(null));
    };
    createEffect(on(() => props.agentId, (id) => {
        setOutcome(null);
        load(id);
    }));

    let unlisten: (() => void) | undefined;
    void listenEvent<any>("memory-adoption-result", (payload) => {
        if (payload?.agent_id !== props.agentId || payload?.kind !== "release") return;
        setWaiting(null);
        setOutcome(releaseOutcome(payload));
        load(props.agentId);
    }).then((u) => {
        unlisten = u;
    });
    onCleanup(() => unlisten?.());

    const folders = () => list()?.folders ?? [];

    const release = async (f: NativeMemoryClaimedFolder) => {
        const l = list();
        if (!l || waiting() !== null) return;
        setOutcome(null);
        setWaiting(f.index);
        try {
            await invokeCommand("memory_release_request", {
                window_label: await getApi().getWindowLabel(),
                agent_id: props.agentId,
                list_id: l.list_id,
                index: f.index,
                summary: { agentName: props.agentName, dir: f.dir ?? undefined },
            });
        } catch (e) {
            setWaiting(null);
            setOutcome({ status: "failed", text: `Couldn't open the confirmation window: ${e}` });
        }
    };

    return (
        <Show when={folders().length > 0 || outcome()}>
            <section class="memory-claims-panel" aria-label="Memory folders this agent claims">
                <Show when={folders().length > 0}>
                    <button type="button" class="memory-adoption-panel-toggle" onClick={() => setOpen(!open())}>
                        <i class={`fa-solid fa-chevron-${open() ? "down" : "right"}`} aria-hidden="true" />
                        Memory folders this agent claims ({folders().length})
                    </button>
                </Show>
                <Show when={open() && folders().length > 0}>
                    <div class="memory-adoption-panel-list">
                        <For each={folders()}>
                            {(f) => (
                                <div class="memory-claims-panel-folder">
                                    <span class="memory-claims-panel-dir" title={f.dir ?? undefined}>
                                        {f.dir ?? "(folder not recorded)"}
                                    </span>
                                    <span class="memory-claims-panel-meta">
                                        {claimedSince(f.claimed_at_ms)}
                                        {f.others > 0 ? ` · shared with ${f.others} other agent${f.others === 1 ? "" : "s"}` : ""}
                                    </span>
                                    <button
                                        type="button"
                                        class="memory-adoption-panel-adopt"
                                        disabled={waiting() !== null}
                                        onClick={() => void release(f)}
                                    >
                                        {waiting() === f.index ? "Waiting…" : "Release…"}
                                    </button>
                                </div>
                            )}
                        </For>
                    </div>
                </Show>
                <Show when={outcome()}>
                    {(o) => <p class={`memory-adoption-panel-outcome is-${o().status}`}>{o().text}</p>}
                </Show>
            </section>
        </Show>
    );
};

MemoryClaimsPanel.displayName = "MemoryClaimsPanel";
