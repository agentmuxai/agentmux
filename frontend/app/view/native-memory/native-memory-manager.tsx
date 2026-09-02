// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NativeMemoryManager — Armory's "Memory → Personal" tab: a grid of agent
 * cards that drills into the same NativeMemoryHistoryPanel a Stash pane's own
 * Memory tab uses. Both read the identical agent:memory:{list,history,diff,
 * revert} RPCs — one source of truth, two entry points (per-agent Stash,
 * cross-agent Armory) — no copy or migration step between them. See
 * docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.3.
 *
 * The agent picker was a `<select>` until 2026-09-01; it's now a card grid
 * mirroring the Agent pane's "My Agents", so you can see which agents have
 * memories without selecting each in turn. See
 * docs/specs/SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md.
 *
 * Deliberately named/placed away from `frontend/app/view/memory/` (Armory's
 * *Bundles* tab — `MemoryManager`, a legacy name predating the
 * Preset→Bundle rename — see that spec's §4.3 "naming collision to avoid").
 * This directory and component are about native memory only.
 */

import { createEffect, createSignal, For, Show, onCleanup, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { NativeMemoryHistoryPanel } from "@/app/view/agent/components/NativeMemoryHistoryPanel";
import { MemoryAgentCard, type MemoryCountState } from "./MemoryAgentCard";
import "./native-memory-manager.scss";

export function NativeMemoryManager(): JSX.Element {
    const [agents] = useAgentDefinitions();
    const [selectedAgent, setSelectedAgent] = createSignal<AgentDefinition | null>(null);
    const [files, setFiles] = createSignal<NativeMemoryFileMeta[]>([]);
    const [selectedFilename, setSelectedFilename] = createSignal<string>("");
    const [filesLoading, setFilesLoading] = createSignal(false);
    const [filesError, setFilesError] = createSignal<string | null>(null);
    let latestRequestId = 0;

    // Per-agent memory counts for the grid, keyed by agent id. Fetched lazily
    // and concurrently once the agent list resolves — one `agent:memory:list`
    // per agent, each settling independently so a single slow or failing
    // agent never blocks the grid's first paint or blanks its siblings.
    const [counts, setCounts] = createSignal<Record<string, MemoryCountState>>({});
    // Generation guard for the whole grid: if `agents()` changes while a batch
    // is in flight, results from the abandoned batch must not paint. Same
    // reasoning as `latestRequestId` below (reagent P2, PR #2678) — N
    // concurrent fetches make out-of-order resolution MORE likely, not less.
    let latestCountsGeneration = 0;

    createEffect(() => {
        const list = agents();
        const generation = ++latestCountsGeneration;
        if (list.length === 0) {
            setCounts({});
            return;
        }
        setCounts(Object.fromEntries(list.map((a) => [a.id, { kind: "loading" } as MemoryCountState])));
        for (const agent of list) {
            RpcApi.NativeMemoryListCommand(TabRpcClient, { agent_id: agent.id })
                .then((res) => {
                    if (generation !== latestCountsGeneration) return;
                    setCounts((prev) => ({ ...prev, [agent.id]: { kind: "count", files: res.files.length } }));
                })
                .catch((e: Error) => {
                    if (generation !== latestCountsGeneration) return;
                    // NOT folded into `count: 0`. agent:memory:list fails with
                    // a hard 500 when the memory dir can't be resolved — the
                    // exact shape of the bug #2901 fixed — so an error must
                    // stay visually distinct from a genuinely empty agent.
                    setCounts((prev) => ({
                        ...prev,
                        [agent.id]: { kind: "error", message: e.message ?? String(e) },
                    }));
                });
        }
        onCleanup(() => {
            // Abandon this batch's results if the effect re-runs.
            latestCountsGeneration++;
        });
    });

    // Re-fetch the file list whenever the selected agent changes; clear the
    // file selection so a stale filename from a different agent can't leak
    // into NativeMemoryHistoryPanel's props.
    //
    // `requestId` guards against a stale response: switching agents twice in
    // quick succession fires two overlapping fetches, and network ordering
    // doesn't guarantee the later request's response arrives last. Only the
    // effect run whose id still matches the latest one is allowed to apply
    // its result — an in-flight response from an abandoned agent selection
    // is silently dropped instead of overwriting the current selection's
    // file list (reagent P2 on PR #2678).
    createEffect(() => {
        const agent = selectedAgent();
        const requestId = ++latestRequestId;
        setSelectedFilename("");
        setFiles([]);
        setFilesError(null);
        if (!agent) return;
        setFilesLoading(true);
        RpcApi.NativeMemoryListCommand(TabRpcClient, { agent_id: agent.id })
            .then((res) => {
                if (requestId !== latestRequestId) return;
                setFiles(res.files);
            })
            .catch((e: Error) => {
                if (requestId !== latestRequestId) return;
                setFilesError(`Failed to list memory files: ${e.message ?? e}`);
            })
            .finally(() => {
                if (requestId !== latestRequestId) return;
                setFilesLoading(false);
            });
    });

    const agentLabel = (a: AgentDefinition) => a.name || a.slug || a.id;

    return (
        <div class="native-memory-manager">
            <Show
                when={selectedAgent()}
                fallback={
                    <div class="native-memory-manager-grid-view">
                        <p class="native-memory-manager-hint">
                            Each agent's own native memory — what it has chosen to remember. Select an
                            agent to browse its files, versions and diffs.
                        </p>
                        <Show
                            when={agents().length > 0}
                            fallback={<div class="native-memory-manager-empty">No agents defined yet.</div>}
                        >
                            <div class="native-memory-manager-grid">
                                <For each={agents()}>
                                    {(agent) => (
                                        <MemoryAgentCard
                                            agent={agent}
                                            count={counts()[agent.id] ?? { kind: "loading" }}
                                            onSelect={setSelectedAgent}
                                        />
                                    )}
                                </For>
                            </div>
                        </Show>
                    </div>
                }
            >
                {(agent) => (
                    <div class="native-memory-manager-detail">
                        <div class="native-memory-manager-header">
                            <button
                                type="button"
                                class="native-memory-manager-back"
                                onClick={() => setSelectedAgent(null)}
                            >
                                ← All agents
                            </button>
                            <span class="native-memory-manager-agent-name">{agentLabel(agent())}</span>

                            <label class="native-memory-manager-field">
                                <span>File</span>
                                <select
                                    value={selectedFilename()}
                                    disabled={filesLoading() || files().length === 0}
                                    onChange={(e) => setSelectedFilename(e.currentTarget.value)}
                                >
                                    <option value="">
                                        {filesLoading()
                                            ? "Loading…"
                                            : files().length === 0
                                              ? "No memory files"
                                              : "Select a file…"}
                                    </option>
                                    <For each={files()}>
                                        {(file) => <option value={file.filename}>{file.filename}</option>}
                                    </For>
                                </select>
                            </label>
                        </div>

                        <Show when={filesError()}>
                            <div class="native-memory-manager-error">{filesError()}</div>
                        </Show>

                        <div class="native-memory-manager-body">
                            <Show
                                when={selectedFilename()}
                                fallback={
                                    <div class="native-memory-manager-empty">
                                        {filesError()
                                            ? "This agent's memory directory could not be read."
                                            : "Pick a memory file to see its version history."}
                                    </div>
                                }
                            >
                                {/* Keyed on agentId:filename so switching either forces a
                                    clean remount — NativeMemoryHistoryPanel's own doc
                                    comment: it does not react to prop changes after
                                    mount by design. */}
                                <Show when={`${agent().id}:${selectedFilename()}`} keyed>
                                    {(_key) => (
                                        <NativeMemoryHistoryPanel
                                            agentId={agent().id}
                                            filename={selectedFilename()}
                                        />
                                    )}
                                </Show>
                            </Show>
                        </div>
                    </div>
                )}
            </Show>
        </div>
    );
}

NativeMemoryManager.displayName = "NativeMemoryManager";
