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
 * The grid gained find/filter + sort as of 2026-09-02, once it routinely
 * rendered 30+ cards in practice — same trigger and interaction pattern as
 * the Agent pane's own filter bar, adapted where the two views genuinely
 * differ (no launch-recency data here; filtering is pure client-side over
 * an already-fully-loaded list, not RPC-paginated). See
 * docs/specs/SPEC_ARMORY_PERSONAL_MEMORY_FILTER_AND_SORT_2026_09_02.md.
 *
 * Deliberately named/placed away from `frontend/app/view/memory/` (Armory's
 * *Bundles* tab — `MemoryManager`, a legacy name predating the
 * Preset→Bundle rename — see that spec's §4.3 "naming collision to avoid").
 * This directory and component are about native memory only.
 */

import { createEffect, createMemo, createSignal, For, onCleanup, Show, untrack, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { NativeMemoryHistoryPanel } from "@/app/view/agent/components/NativeMemoryHistoryPanel";
import { MemoryAgentCard, type MemoryCountState } from "./MemoryAgentCard";
import {
    DEFAULT_MEMORY_SORT,
    MemoryAgentFilterBar,
    type MemoryAgentSortOption,
} from "./MemoryAgentFilterBar";
import "./native-memory-manager.scss";

const MEMORY_SORT_STORAGE_KEY = "nativeMemory:sortBy";

/** "name" is only the fallback for a never-set preference — once a sort has
 *  been chosen it's read back here on every load, i.e. the operative
 *  default is "last used", matching AgentPickerFilterBar's own precedent
 *  (`loadStoredSort` in AgentPicker.tsx). try/catch mirrors that helper too:
 *  localStorage can throw in some embedding contexts (private browsing,
 *  storage quota), and a sort preference is never worth a hard failure. */
function loadStoredMemorySort(): MemoryAgentSortOption {
    try {
        const raw = localStorage.getItem(MEMORY_SORT_STORAGE_KEY);
        if (raw === "name" || raw === "count" || raw === "provider") return raw;
    } catch {
        // ignore — fall through to the default
    }
    return DEFAULT_MEMORY_SORT;
}

function storeMemorySort(sort: MemoryAgentSortOption): void {
    try {
        localStorage.setItem(MEMORY_SORT_STORAGE_KEY, sort);
    } catch {
        // best-effort only — a failed write just means the preference
        // doesn't survive to the next Armory open, not a functional break
    }
}

/**
 * Comparator for the grid's `count` sort. `loading`/`error` cards sort
 * AFTER every resolved count (in that order — loading before error), each
 * group alphabetical by name internally: a numeric "most files" sort has no
 * correct answer for a count that isn't known yet or failed to resolve, so
 * burying those at the bottom is the intentional behavior, not a fallback
 * bug. The card itself still labels an error distinctly from a zero count —
 * this ordering never collapses that distinction, it just decides where an
 * unresolved card sits in the list.
 */
function compareByCount(
    a: AgentDefinition,
    b: AgentDefinition,
    counts: Record<string, MemoryCountState>,
): number {
    const rank = (id: string): number => {
        const c = counts[id];
        if (c?.kind === "count") return 0;
        if (c?.kind === "loading") return 1;
        return 2; // "error", or not yet in the map at all
    };
    const ra = rank(a.id);
    const rb = rank(b.id);
    if (ra !== rb) return ra - rb;
    if (ra === 0) {
        const fa = (counts[a.id] as { kind: "count"; files: number }).files;
        const fb = (counts[b.id] as { kind: "count"; files: number }).files;
        if (fa !== fb) return fb - fa;
    }
    return agentLabel(a).localeCompare(agentLabel(b), undefined, { sensitivity: "base" });
}

function compareAgents(
    sort: MemoryAgentSortOption,
    a: AgentDefinition,
    b: AgentDefinition,
    counts: Record<string, MemoryCountState>,
): number {
    switch (sort) {
        case "count":
            return compareByCount(a, b, counts);
        case "provider": {
            const providerDiff = a.provider.localeCompare(b.provider, undefined, { sensitivity: "base" });
            if (providerDiff !== 0) return providerDiff;
            return agentLabel(a).localeCompare(agentLabel(b), undefined, { sensitivity: "base" });
        }
        case "name":
        default:
            return agentLabel(a).localeCompare(agentLabel(b), undefined, { sensitivity: "base" });
    }
}

const agentLabel = (a: AgentDefinition) => a.name || a.slug || a.id;

export function NativeMemoryManager(): JSX.Element {
    // `agentsLoading` is why this hook returns a pair: `agents()` is `[]` both
    // before the first ListAgentDefinitions settles AND when there genuinely
    // are none. Without the flag the grid flashes "No agents defined yet" on
    // every mount (Codex P2, PR #2917) — the same reason AgentPicker consumes
    // it.
    const [agents, agentsLoading] = useAgentDefinitions();
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

    // Keyed on the SET of agent ids, not on `agents()` itself. `agents()` is a
    // brand-new array on every `agents:changed` event — which fires for any
    // agent create/update/delete/template op anywhere in the app, not just
    // ones that change this tab's agent set. Depending on the array reference
    // meant an unrelated edit elsewhere reset every card to "Loading…" and
    // refetched counts that had already resolved (ReAgent P2, PR #2917).
    const agentIdsKey = createMemo(() =>
        agents()
            .map((a) => a.id)
            .sort()
            .join(" "),
    );

    // Shared by the id-set effect below (new agents) AND the reactive
    // agent:memory:changed handler further down (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md)
    // — one RPC-call-then-setCounts implementation, not two copies that could
    // drift. Always overwrites, even for an agent not currently `undefined`
    // in the map — the reactive path's whole point is refreshing an agent
    // whose count was already resolved (or errored) before this call.
    const fetchCountFor = (agent: AgentDefinition) => {
        setCounts((prev) => ({ ...prev, [agent.id]: { kind: "loading" } as MemoryCountState }));
        RpcApi.NativeMemoryListCommand(TabRpcClient, { agent_id: agent.id })
            .then((res) => {
                // Guarded per-agent rather than by a batch generation: a
                // response is only stale if its agent is gone, and each
                // fetch now settles into a shared, incrementally-built map
                // (the old whole-batch reset is what caused the flicker).
                setCounts((prev) =>
                    prev[agent.id] === undefined
                        ? prev
                        : { ...prev, [agent.id]: { kind: "count", files: res.files.length } },
                );
            })
            .catch((e: Error) => {
                // NOT folded into `count: 0`. agent:memory:list fails with
                // a hard 500 when the memory dir can't be resolved — the
                // exact shape of the bug #2901 fixed — so an error must
                // stay visually distinct from a genuinely empty agent.
                setCounts((prev) =>
                    prev[agent.id] === undefined
                        ? prev
                        : { ...prev, [agent.id]: { kind: "error", message: e.message ?? String(e) } },
                );
            });
    };

    createEffect(() => {
        agentIdsKey();
        // Read the list itself untracked: the memo above is the only intended
        // dependency, so a new-but-equivalent array can't re-trigger this.
        const list = untrack(() => agents());
        const present = new Set(list.map((a) => a.id));

        // Drop entries for agents that no longer exist, and keep every count
        // already resolved — only genuinely new agents are fetched below.
        setCounts((prev) => Object.fromEntries(Object.entries(prev).filter(([id]) => present.has(id))));

        const missing = list.filter((a) => untrack(() => counts())[a.id] === undefined);
        for (const agent of missing) fetchCountFor(agent);
    });

    // Grid find/filter + sort (SPEC_ARMORY_PERSONAL_MEMORY_FILTER_AND_SORT_2026_09_02.md).
    // Session-only, like AgentPicker's own filterQuery — a stale
    // hidden-by-filter grid on next open would be more surprising than a
    // cleared filter. Sort choice is the one persisted piece (see
    // loadStoredMemorySort's own doc comment).
    const [nameFilter, setNameFilter] = createSignal("");
    const [onlyWithMemories, setOnlyWithMemories] = createSignal(false);
    const [sortBy, setSortByState] = createSignal<MemoryAgentSortOption>(loadStoredMemorySort());
    const setSortBy = (sort: MemoryAgentSortOption) => {
        setSortByState(sort);
        storeMemorySort(sort);
    };

    // Pure client-side filter/sort over already-fully-loaded data
    // (`agents()`/`counts()`) — unlike MyAgentsList, this list isn't
    // RPC-paginated, so there's nothing to debounce or refetch here.
    const visibleAgents = createMemo(() => {
        const query = nameFilter().trim().toLowerCase();
        const onlyMemories = onlyWithMemories();
        const currentCounts = counts();
        const filtered = agents().filter((agent) => {
            if (query && !agentLabel(agent).toLowerCase().includes(query)) return false;
            // Never hides a `loading`/`error` card — only a resolved `count: 0`
            // counts as "no memories" for this toggle (see the spec's own
            // rationale: a slow or failing fetch shouldn't make a card vanish
            // out from under the user mid-load).
            if (onlyMemories) {
                const c = currentCounts[agent.id];
                if (c?.kind === "count" && c.files === 0) return false;
            }
            return true;
        });
        return filtered.slice().sort((a, b) => compareAgents(sortBy(), a, b, currentCounts));
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

    // Reactive re-fetch of the OPEN detail view's file list, triggered by
    // agent:memory:changed for the currently-selected agent
    // (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md). Deliberately does NOT
    // clear `selectedFilename()` the way the selectedAgent()-keyed effect
    // above does on an actual agent SWITCH — a live write to the file
    // you're already looking at should update in place, not kick you back
    // to "pick a file." Only clears it if the previously-selected filename
    // genuinely no longer exists in the refreshed list (e.g. the file that
    // changed was a delete).
    const refetchSelectedAgentFiles = (agent: AgentDefinition) => {
        const requestId = ++latestRequestId;
        RpcApi.NativeMemoryListCommand(TabRpcClient, { agent_id: agent.id })
            .then((res) => {
                if (requestId !== latestRequestId) return;
                setFiles(res.files);
                setFilesError(null);
                const current = selectedFilename();
                if (current && !res.files.some((f) => f.filename === current)) {
                    setSelectedFilename("");
                }
            })
            .catch((e: Error) => {
                if (requestId !== latestRequestId) return;
                setFilesError(`Failed to list memory files: ${e.message ?? e}`);
            });
    };

    // Reactive grid + detail updates: subscribe to agent:memory:changed for
    // every agent currently in the grid, re-subscribing whenever the agent
    // SET changes (same agentIdsKey dependency the count-fetch effect above
    // uses, for the same reason — a brand-new-but-equivalent agents() array
    // must not tear down and rebuild every subscription).
    // SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md.
    //
    // Debounced per agent id (~250ms): a burst of rapid writes to the same
    // agent (e.g. a script issuing several MemoryWrite calls in a loop)
    // coalesces into one refetch instead of one RPC round-trip per write.
    // Keyed per agent id so a burst on one agent never delays another's.
    const debounceTimers = new Map<string, ReturnType<typeof setTimeout>>();
    createEffect(() => {
        agentIdsKey();
        const list = untrack(() => agents());

        const unsub = waveEventSubscribe(
            ...list.map((agent) => ({
                eventType: `agent:memory:changed:${agent.id}`,
                handler: () => {
                    const existing = debounceTimers.get(agent.id);
                    if (existing !== undefined) clearTimeout(existing);
                    debounceTimers.set(
                        agent.id,
                        setTimeout(() => {
                            debounceTimers.delete(agent.id);
                            fetchCountFor(agent);
                            if (untrack(selectedAgent)?.id === agent.id) {
                                refetchSelectedAgentFiles(agent);
                            }
                        }, 250),
                    );
                },
            })),
        );

        onCleanup(() => {
            unsub();
            for (const timer of debounceTimers.values()) clearTimeout(timer);
            debounceTimers.clear();
        });
    });

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
                            fallback={
                                <div class="native-memory-manager-empty">
                                    {agentsLoading() ? "Loading agents…" : "No agents defined yet."}
                                </div>
                            }
                        >
                            <MemoryAgentFilterBar
                                value={nameFilter}
                                onInput={setNameFilter}
                                onClear={() => setNameFilter("")}
                                onlyWithMemories={onlyWithMemories}
                                onOnlyWithMemoriesChange={setOnlyWithMemories}
                                sortBy={sortBy}
                                onSortChange={setSortBy}
                            />
                            <Show
                                when={visibleAgents().length > 0}
                                fallback={
                                    <div class="native-memory-manager-empty">
                                        {/* Distinct from "No agents defined yet." above — that
                                            means zero agents exist at all; this means the fetch
                                            succeeded but the current filter(s) matched nothing. */}
                                        {nameFilter()
                                            ? `No agents match "${nameFilter()}"`
                                            : "No agents match the current filter."}
                                    </div>
                                }
                            >
                                <div class="native-memory-manager-grid">
                                    <For each={visibleAgents()}>
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
