// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NativeMemoryManager — the Memory pane's Personal section (Personal Memory): a grid of agent
 * cards that drills into a file's full view (NativeMemoryFileView), built
 * from the MemoryHistory/MemoryContent pieces, reading the agent:memory:{list,
 * history,diff,revert} RPCs. See
 * docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.3.
 *
 * The agent picker was a `<select>` until 2026-09-01; it's now a card grid
 * mirroring the Agent pane's "My Agents", so you can see which agents have
 * memories without selecting each in turn. See
 * docs/specs/SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md.
 *
 * The FILE picker was a `<select>` until 2026-09-04; the detail view is now
 * two screens — a file tile grid, then the history panel — and the back
 * button steps back one level rather than always returning to the agent grid.
 * See docs/specs/SPEC_ARMORY_PERSONAL_MEMORY_FILE_TILES_2026_09_04.md.
 *
 * The full view became editable, with its editor pinned to the bottom, on
 * 2026-09-24 (NativeMemoryFileView) — and no longer remounts on every live
 * change, so an open draft survives one. See
 * docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3/§2.4.
 *
 * The grid gained find/filter + sort as of 2026-09-02, once it routinely
 * rendered 30+ cards in practice — same trigger and interaction pattern as
 * the Agent pane's own filter bar, adapted where the two views genuinely
 * differ (no launch-recency data here; filtering is pure client-side over
 * an already-fully-loaded list, not RPC-paginated). See
 * docs/specs/SPEC_ARMORY_PERSONAL_MEMORY_FILTER_AND_SORT_2026_09_02.md.
 *
 * Deliberately named/placed away from `frontend/app/view/memory/` (then
 * the *Bundles* manager — `MemoryManager`, a legacy name predating the
 * Preset→Bundle rename — see that spec's §4.3 "naming collision to avoid").
 * This directory and component are about native memory only.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { onMuxEvent } from "@/app/store/mps";
import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import type { AgentDefinition, NativeMemoryFileMeta } from "@/app/store/rpc-api";
import {
    createEffect,
    createMemo,
    createSignal,
    For,
    onCleanup,
    Show,
    untrack,
    type Accessor,
    type JSX,
    type Setter,
} from "solid-js";
import { MemoryAgentCard, type MemoryCountState } from "./MemoryAgentCard";
import { AgentLaunchCost } from "./AgentLaunchCost";
import { deliveredTotal, windowFromMetas } from "./agent-launch-cost";
import { WpsEvent } from "@/app/store/mps-events";
import type { LaunchCost } from "@/types/rpc/LaunchCost";
import type { AgentOpenPane } from "@/types/rpc/AgentOpenPane";
import { MOS } from "@/app/store/global";
import { DEFAULT_MEMORY_SORT, MemoryAgentFilterBar, type MemoryAgentSortOption } from "./MemoryAgentFilterBar";
import { fileDistribution, fileDistributionLabel, type FleetAgent } from "./memory-aggregates";
import { MemoryFleetSummary } from "./MemoryFleetSummary";
import { MemoryFileCard } from "./MemoryFileCard";
import { resolveProviderAlias } from "@/app/view/agent/providers/catalog";
import { NativeMemoryFileView } from "./NativeMemoryFileView";
import { MemoryAdoptionPanel } from "./MemoryAdoptionPanel";
import { MemoryClaimsPanel } from "./MemoryClaimsPanel";
import { NativeMemoryCreate } from "./NativeMemoryCreate";
import { MEMORY_AGENT_KEY } from "@/app/store/section-panes";
import "./native-memory-manager.scss";
import { matchesEveryWord } from "@/app/util/fuzzysearch";
import { Button } from "@/app/element/ui";
import { tip } from "@/app/element/tip";

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
        if (raw === "tokens" || raw === "name" || raw === "count" || raw === "provider") return raw;
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
        // doesn't survive to the next Memory open, not a functional break
    }
}

/** Fewer files than this, and a median and p90 say nothing a glance at the
 *  tiles doesn't (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.5). */
const MIN_FILES_FOR_DISTRIBUTION = 5;

/**
 * Comparator for the grid's `count` and `tokens` sorts (largest first, by
 * `key`). `loading`/`error` cards sort AFTER every resolved count (in that order — loading before error), each
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
    key: "files" | "tokens"
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
        const fa = (counts[a.id] as Extract<MemoryCountState, { kind: "count" }>)[key];
        const fb = (counts[b.id] as Extract<MemoryCountState, { kind: "count" }>)[key];
        if (fa !== fb) return fb - fa;
    }
    return agentLabel(a).localeCompare(agentLabel(b), undefined, { sensitivity: "base" });
}

function compareAgents(
    sort: MemoryAgentSortOption,
    a: AgentDefinition,
    b: AgentDefinition,
    counts: Record<string, MemoryCountState>
): number {
    switch (sort) {
        case "tokens":
            return compareByCount(a, b, counts, "tokens");
        case "count":
            return compareByCount(a, b, counts, "files");
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

/** The pane hosting this section: where an entry point (`openMemory`) names the agent to show. */
interface NativeMemoryPane {
    meta: Accessor<MetaType | undefined>;
    setMeta: (patch: Record<string, unknown>) => void;
}

/** The pane's agent, the list it comes from, and the request for one that the pane carries. */
interface SelectionState {
    props: { pane?: NativeMemoryPane };
    agents: Accessor<AgentDefinition[]>;
    agentsLoading: Accessor<boolean>;
    selectedAgent: Accessor<AgentDefinition | null>;
    setSelectedAgentState: Setter<AgentDefinition | null>;
}

/** The open agent's files, its open file, and what a refresh of them needs. */
interface FilesState {
    files: Accessor<NativeMemoryFileMeta[]>;
    setFiles: Setter<NativeMemoryFileMeta[]>;
    unverified: Accessor<boolean>;
    setUnverified: Setter<boolean>;
    selectedFilename: Accessor<string>;
    setSelectedFilename: Setter<string>;
    filesLoading: Accessor<boolean>;
    setFilesLoading: Setter<boolean>;
    filesError: Accessor<string | null>;
    setFilesError: Setter<string | null>;
    refreshNonce: Accessor<number>;
    setRefreshNonce: Setter<number>;
    detailDirty: Accessor<boolean>;
    setDetailDirty: Setter<boolean>;
    sortedFiles: Accessor<NativeMemoryFileMeta[]>;
    latestRequestId: number;
}

/** The grid: every agent's count and launch cost, the open panes, the filter, and what each request in flight is. */
interface GridState {
    counts: Accessor<Record<string, MemoryCountState>>;
    setCounts: Setter<Record<string, MemoryCountState>>;
    launchCosts: Accessor<Record<string, LaunchCost>>;
    setLaunchCosts: Setter<Record<string, LaunchCost>>;
    openPanes: Accessor<AgentOpenPane[]>;
    setOpenPanes: Setter<AgentOpenPane[]>;
    agentIdsKey: Accessor<string>;
    nameFilter: Accessor<string>;
    setNameFilter: Setter<string>;
    onlyWithMemories: Accessor<boolean>;
    setOnlyWithMemories: Setter<boolean>;
    sortBy: Accessor<MemoryAgentSortOption>;
    setSortByState: Setter<MemoryAgentSortOption>;
    visibleAgents: Accessor<AgentDefinition[]>;
    fleetAgents: Accessor<FleetAgent[]>;
    countRequestGeneration: Map<string, number>;
    launchRequestGeneration: Map<string, number>;
    panesSeq: number;
    panesApplied: number;
    debounceTimers: Map<string, ReturnType<typeof setTimeout>>;
}

type ManagerState = SelectionState & FilesState & GridState;

function createFilesState(): FilesState {
    const [files, setFiles] = createSignal<NativeMemoryFileMeta[]>([]);
    // The folder was found by a guess, not the agent's own launch
    // (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2): read-only.
    const [unverified, setUnverified] = createSignal(false);
    const [selectedFilename, setSelectedFilename] = createSignal<string>("");
    const [filesLoading, setFilesLoading] = createSignal(false);
    const [filesError, setFilesError] = createSignal<string | null>(null);
    // Bumped on every reactive refresh of the open agent's files so the open
    // full view reloads in place even when agentId:filename is unchanged —
    // see refetchSelectedAgentFiles's own comment (Codex P1, PR #2932). It
    // used to force a REMOUNT; now it's a prop, so an open draft survives.
    const [refreshNonce, setRefreshNonce] = createSignal(0);
    // Whether the open full view holds unsaved edits — guards Back, and
    // stops a refresh that no longer lists the file from yanking the view
    // (and the draft) away. SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4.
    const [detailDirty, setDetailDirty] = createSignal(false);

    // Index file first, then largest first (tokens), then alphabetical
    // (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.1). The list RPC's own order is a
    // plain directory listing, which puts MEMORY.md first only by the accident
    // of uppercase sorting ahead of lowercase in ASCII — rename the index and
    // it lands mid-grid. The index is the file loaded into every session, so
    // it is the one tile whose position is worth pinning explicitly rather
    // than leaving to collation.
    const sortedFiles = createMemo(() =>
        [...files()].sort((a, b) => {
            if (a.is_index !== b.is_index) return a.is_index ? -1 : 1;
            const byTokens = (b.tokens_est ?? 0) - (a.tokens_est ?? 0);
            if (byTokens !== 0) return byTokens;
            return a.filename.localeCompare(b.filename, undefined, { sensitivity: "base" });
        })
    );

    return {
        files,
        setFiles,
        unverified,
        setUnverified,
        selectedFilename,
        setSelectedFilename,
        filesLoading,
        setFilesLoading,
        filesError,
        setFilesError,
        refreshNonce,
        setRefreshNonce,
        detailDirty,
        setDetailDirty,
        sortedFiles,
        latestRequestId: 0,
    };
}

function createVisibleAgents(
    agents: Accessor<AgentDefinition[]>,
    nameFilter: Accessor<string>,
    onlyWithMemories: Accessor<boolean>,
    sortBy: Accessor<MemoryAgentSortOption>,
    counts: Accessor<Record<string, MemoryCountState>>
): Accessor<AgentDefinition[]> {
    // Pure client-side filter/sort over already-fully-loaded data
    // (`agents()`/`counts()`) — unlike MyAgentsList, this list isn't
    // RPC-paginated, so there's nothing to debounce or refetch here.
    return createMemo(() => {
        const query = nameFilter();
        const onlyMemories = onlyWithMemories();
        const currentCounts = counts();
        const filtered = agents().filter((agent) => {
            if (!matchesEveryWord(query, agentLabel(agent))) return false;
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
}

function createFleetAgents(
    agents: Accessor<AgentDefinition[]>,
    counts: Accessor<Record<string, MemoryCountState>>
): Accessor<FleetAgent[]> {
    // Every agent whose count has arrived, for the fleet line (spec §5.5).
    return createMemo(() => {
        const c = counts();
        return agents().flatMap((a) => {
            const n = c[a.id];
            return n?.kind === "count" ? [{ id: a.id, name: agentLabel(a), tokens: n.tokens }] : [];
        });
    });
}

function createGridState(agents: Accessor<AgentDefinition[]>): GridState {
    // Per-agent memory counts for the grid, keyed by agent id. Fetched lazily
    // and concurrently once the agent list resolves — one `agent:memory:list`
    // per agent, each settling independently so a single slow or failing
    // agent never blocks the grid's first paint or blanks its siblings.
    const [counts, setCounts] = createSignal<Record<string, MemoryCountState>>({});
    // What each agent's next launch delivers (`memory:launchcost`), fetched
    // with its count; and the agent panes open now, whose block meta carries
    // the context window a budget is measured against
    // (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.3).
    const [launchCosts, setLaunchCosts] = createSignal<Record<string, LaunchCost>>({});
    const [openPanes, setOpenPanes] = createSignal<AgentOpenPane[]>([]);

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
            .join(" ")
    );

    // Per-agent request generation for fetchCountFor (#2932):
    // before the reactive agent:memory:changed handler existed, an
    // agent's count was only ever fetched ONCE per appearance in the grid —
    // "is this agent still present" was the only staleness question that
    // could arise. Now a change event can call fetchCountFor again while an
    // earlier call for the SAME agent is still in flight (e.g. the initial
    // fetch is unusually slow, or two events land close together), and
    // without this, an older response resolving after a newer one would
    // overwrite the fresher result and leave the card stale until another
    // event or a remount.
    const countRequestGeneration = new Map<string, number>();
    const launchRequestGeneration = new Map<string, number>();
    const debounceTimers = new Map<string, ReturnType<typeof setTimeout>>();

    // Grid find/filter + sort (SPEC_ARMORY_PERSONAL_MEMORY_FILTER_AND_SORT_2026_09_02.md).
    // Session-only, like AgentPicker's own filterQuery — a stale
    // hidden-by-filter grid on next open would be more surprising than a
    // cleared filter. Sort choice is the one persisted piece (see
    // loadStoredMemorySort's own doc comment).
    const [nameFilter, setNameFilter] = createSignal("");
    const [onlyWithMemories, setOnlyWithMemories] = createSignal(false);
    const [sortBy, setSortByState] = createSignal<MemoryAgentSortOption>(loadStoredMemorySort());
    const visibleAgents = createVisibleAgents(agents, nameFilter, onlyWithMemories, sortBy, counts);
    const fleetAgents = createFleetAgents(agents, counts);

    return {
        counts,
        setCounts,
        launchCosts,
        setLaunchCosts,
        openPanes,
        setOpenPanes,
        agentIdsKey,
        nameFilter,
        setNameFilter,
        onlyWithMemories,
        setOnlyWithMemories,
        sortBy,
        setSortByState,
        visibleAgents,
        fleetAgents,
        countRequestGeneration,
        launchRequestGeneration,
        panesSeq: 0,
        panesApplied: 0,
        debounceTimers,
    };
}

function createManagerState(props: { pane?: NativeMemoryPane }): ManagerState {
    // `agentsLoading` is why this hook returns a pair: `agents()` is `[]` both
    // before the first ListAgentDefinitions settles AND when there genuinely
    // are none. Without the flag the grid flashes "No agents defined yet" on
    // every mount (Codex P2, PR #2917) — the same reason AgentPicker consumes
    // it.
    const [agents, agentsLoading] = useAgentDefinitions();
    const [selectedAgent, setSelectedAgentState] = createSignal<AgentDefinition | null>(null);
    return {
        props,
        agents,
        agentsLoading,
        selectedAgent,
        setSelectedAgentState,
        ...createFilesState(),
        ...createGridState(agents),
    };
}

export function NativeMemoryManager(props: { pane?: NativeMemoryPane }): JSX.Element {
    const s = createManagerState(props);
    watchPaneRequest(s);
    subscribeToChanges(s);
    watchAgentSet(s);
    watchSelectedAgentFiles(s);
    watchMemoryChanges(s);

    return (
        <div class="native-memory-manager">
            <Show when={s.selectedAgent()} fallback={renderAgentGrid(s)}>
                {(agent) => renderDetail(s, agent)}
            </Show>
        </div>
    );
}

// Kept in the pane's meta too, so `openMemory("personal", { agentId })` can pick the agent,
// including in a pane already open on another one.
function setSelectedAgent(s: ManagerState, agent: AgentDefinition | null): void {
    s.setSelectedAgentState(agent);
    s.props.pane?.setMeta({ [MEMORY_AGENT_KEY]: agent?.id ?? null });
}

function setSortBy(s: ManagerState, sort: MemoryAgentSortOption): void {
    s.setSortByState(sort);
    storeMemorySort(sort);
}

function watchPaneRequest(s: ManagerState): void {
    createEffect(() => {
        const id = s.props.pane?.meta()?.[MEMORY_AGENT_KEY];
        if (typeof id !== "string" || id === "") return;
        const wanted = s.agents().find((a) => a.id === id);
        if (!wanted || untrack(s.selectedAgent)?.id === id) return;
        // Unsaved edits in the open file stay: the same confirm Back asks.
        if (untrack(s.detailDirty) && !window.confirm("Discard your unsaved changes?")) {
            // Declined: the pane's request is withdrawn, so a later refresh doesn't ask again.
            s.props.pane?.setMeta({ [MEMORY_AGENT_KEY]: untrack(s.selectedAgent)?.id ?? null });
            return;
        }
        s.setSelectedAgentState(wanted);
    });
}

function windowFor(s: ManagerState, agentId: string) {
    return windowFromMetas(
        s.openPanes()
            .filter((p) => p.agent_id === agentId)
            .map((p) => MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", p.block_id))()?.meta as Record<string, unknown>)
    );
}

// Shared by the id-set effect below (new agents) AND the reactive
// agent:memory:changed handler further down (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md)
// — one RPC-call-then-setCounts implementation, not two copies that could
// drift. Always overwrites, even for an agent not currently `undefined`
// in the map — the reactive path's whole point is refreshing an agent
// whose count was already resolved (or errored) before this call.
function fetchCountFor(s: ManagerState, agent: AgentDefinition): void {
    const generation = (s.countRequestGeneration.get(agent.id) ?? 0) + 1;
    s.countRequestGeneration.set(agent.id, generation);
    // Only flash to "loading" for a genuinely new agent — no resolved
    // state yet to show in the meantime. ReAgent (PR #2932): the
    // reactive agent:memory:changed handler also calls this function,
    // and unconditionally overwriting to "loading" made an
    // already-resolved card visibly flicker back to "Loading…" on
    // every write, rather than updating quietly in place — inconsistent
    // with refetchSelectedAgentFiles's own deliberate choice not to
    // touch filesLoading for the identical reason, and with this
    // feature's own stated design intent of a quiet per-card refresh.
    s.setCounts((prev) =>
        prev[agent.id] === undefined ? { ...prev, [agent.id]: { kind: "loading" } as MemoryCountState } : prev
    );
    RpcApi.NativeMemoryListCommand(TabRpcClient, { agent_id: agent.id })
        .then((res) => {
            // Guarded by BOTH "agent still present" and "still the latest
            // request for this agent" — a response is stale if either its
            // agent is gone, or a newer fetchCountFor call for the same
            // agent has since superseded it.
            if (s.countRequestGeneration.get(agent.id) !== generation) return;
            s.setCounts((prev) =>
                prev[agent.id] === undefined
                    ? prev
                    : {
                          ...prev,
                          [agent.id]: {
                              kind: "count",
                              files: res.files.length,
                              tokens: res.files.reduce((n, f) => n + (f.tokens_est ?? 0), 0),
                          },
                      }
            );
        })
        .catch((e: Error) => {
            if (s.countRequestGeneration.get(agent.id) !== generation) return;
            // NOT folded into `count: 0`. agent:memory:list fails with
            // a hard 500 when the memory dir can't be resolved — the
            // exact shape of the bug #2901 fixed — so an error must
            // stay visually distinct from a genuinely empty agent.
            s.setCounts((prev) =>
                prev[agent.id] === undefined
                    ? prev
                    : { ...prev, [agent.id]: { kind: "error", message: e.message ?? String(e) } }
            );
        });
    fetchLaunchCostFor(s, agent);
}

// An agent's launch cost also depends on Global Memory, bundles, its own
// content and skills, so it has its own request generation and is
// refetched for every agent when any of those change.
function fetchLaunchCostFor(s: ManagerState, agent: AgentDefinition): void {
    const generation = (s.launchRequestGeneration.get(agent.id) ?? 0) + 1;
    s.launchRequestGeneration.set(agent.id, generation);
    // No launch line rather than a second error: the count line already
    // says when an agent can't be read.
    RpcApi.MemoryLaunchCostCommand(TabRpcClient, { agent_id: agent.id })
        .then((cost) => {
            if (s.launchRequestGeneration.get(agent.id) !== generation) return;
            s.setLaunchCosts((prev) => ({ ...prev, [agent.id]: cost }));
        })
        .catch(() => {});
}

function refetchLaunchCosts(s: ManagerState): void {
    untrack(s.agents).forEach((agent) => fetchLaunchCostFor(s, agent));
}

// The open panes, for their windows: refetched when a pane opens or
// closes anywhere (tracked blocks), and when the user comes back to this
// window, as AgentPicker does. A reply older than one already applied is
// dropped.
function fetchOpenPanes(s: ManagerState): void {
    const seq = ++s.panesSeq;
    RpcApi.AgentOpenPanesCommand(TabRpcClient, {})
        .then((res) => {
            if (seq <= s.panesApplied) return;
            s.panesApplied = seq;
            s.setOpenPanes(res.panes);
        })
        .catch(() => {});
}

function subscribeToChanges(s: ManagerState): void {
    onCleanup(onMuxEvent(WpsEvent.MemoriesChanged, () => refetchLaunchCosts(s)));
    onCleanup(onMuxEvent(WpsEvent.AgentContentChanged, () => refetchLaunchCosts(s)));
    onCleanup(onMuxEvent(WpsEvent.SkillsChanged, () => refetchLaunchCosts(s)));
    onCleanup(onMuxEvent(WpsEvent.AgentSkillsChanged, () => refetchLaunchCosts(s)));
    // An agent's own edits (its folder, its kind) change its startup file and
    // which Operator Config entries it gets.
    onCleanup(onMuxEvent(WpsEvent.AgentsChanged, () => refetchLaunchCosts(s)));
    onCleanup(onMuxEvent(WpsEvent.ProcessBrokerTrackedBlocksChanged, () => fetchOpenPanes(s)));
    // Coming back to this window also refetches the launch costs: an agent's
    // startup file can be created or edited on disk, and no event says so
    // (it decides `delivered_by_file`).
    const onFocus = () => {
        fetchOpenPanes(s);
        refetchLaunchCosts(s);
    };
    window.addEventListener("focus", onFocus);
    onCleanup(() => window.removeEventListener("focus", onFocus));
}

function watchAgentSet(s: ManagerState): void {
    createEffect(() => {
        s.agentIdsKey();
        // Read the list itself untracked: the agentIdsKey memo is the only intended
        // dependency, so a new-but-equivalent array can't re-trigger this.
        const list = untrack(() => s.agents());
        const present = new Set(list.map((a) => a.id));

        // Drop entries for agents that no longer exist, and keep every count
        // already resolved — only genuinely new agents are fetched below.
        s.setCounts((prev) => Object.fromEntries(Object.entries(prev).filter(([id]) => present.has(id))));
        s.setLaunchCosts((prev) => Object.fromEntries(Object.entries(prev).filter(([id]) => present.has(id))));
        fetchOpenPanes(s);
        for (const id of s.countRequestGeneration.keys()) {
            if (!present.has(id)) s.countRequestGeneration.delete(id);
        }
        for (const id of s.launchRequestGeneration.keys()) {
            if (!present.has(id)) s.launchRequestGeneration.delete(id);
        }

        const missing = list.filter((a) => untrack(() => s.counts())[a.id] === undefined);
        for (const agent of missing) fetchCountFor(s, agent);
    });
}

function watchSelectedAgentFiles(s: ManagerState): void {
    // Re-fetch the file list whenever the selected agent changes; clear the
    // file selection so a stale filename from a different agent can't leak
    // into NativeMemoryFileView's props.
    //
    // `requestId` guards against a stale response: switching agents twice in
    // quick succession fires two overlapping fetches, and network ordering
    // doesn't guarantee the later request's response arrives last. Only the
    // effect run whose id still matches the latest one is allowed to apply
    // its result — an in-flight response from an abandoned agent selection
    // is silently dropped instead of overwriting the current selection's
    // file list (reagent P2 on PR #2678).
    createEffect(() => {
        const agent = s.selectedAgent();
        const requestId = ++s.latestRequestId;
        s.setSelectedFilename("");
        s.setFiles([]);
        s.setUnverified(false);
        s.setFilesError(null);
        if (!agent) return;
        s.setFilesLoading(true);
        // Its launch cost too, so the breakdown it opens on is current.
        untrack(() => fetchLaunchCostFor(s, agent));
        RpcApi.NativeMemoryListCommand(TabRpcClient, { agent_id: agent.id })
            .then((res) => {
                if (requestId !== s.latestRequestId) return;
                s.setFiles(res.files);
                s.setUnverified(res.unverified === true);
            })
            .catch((e: Error) => {
                if (requestId !== s.latestRequestId) return;
                s.setFilesError(`Failed to list memory files: ${e.message ?? e}`);
            })
            .finally(() => {
                if (requestId !== s.latestRequestId) return;
                s.setFilesLoading(false);
            });
    });
}

// Reactive re-fetch of the OPEN detail view's file list, triggered by
// agent:memory:changed for the currently-selected agent
// (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md). Deliberately does NOT
// clear `selectedFilename()` the way the selectedAgent()-keyed effect
// (watchSelectedAgentFiles) does on an actual agent SWITCH — a live write to the file
// you're already looking at should update in place, not kick you back
// to "pick a file." Only clears it if the previously-selected filename
// genuinely no longer exists in the refreshed list (e.g. the file that
// changed was a delete).
//
// Codex P2 (PR #2932): this must also own `filesLoading`, not just
// `files`. It shares `latestRequestId` with the selectedAgent()-keyed
// effect (watchSelectedAgentFiles) — if this call's requestId supersedes an initial fetch
// still in flight (a change event landing within the debounce window of
// an agent switch), that effect's own `.finally()` skips
// `setFilesLoading(false)` (guarded by the same requestId check), and
// this function previously never touched the flag either — leaving the
// selector permanently disabled/showing "Loading…" even after this
// fresher request succeeds.
function refetchSelectedAgentFiles(s: ManagerState, agent: AgentDefinition): void {
    const requestId = ++s.latestRequestId;
    RpcApi.NativeMemoryListCommand(TabRpcClient, { agent_id: agent.id })
        .then((res) => {
            if (requestId !== s.latestRequestId) return;
            s.setFiles(res.files);
            s.setUnverified(res.unverified === true);
            s.setFilesError(null);
            // Codex P1 (PR #2932): the full view must pick up a live
            // write to the file you're already looking at, even though
            // the selected FILENAME is unchanged. This used to force a
            // remount (the view was keyed on it); since 2026-09-24 the
            // view takes it as a prop and reloads history + content in
            // place, so an open draft isn't wiped — its draft model
            // raises a banner instead, only when the file's hash really
            // moved (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4).
            // Bumped unconditionally on every successful refresh, not
            // just when the file list itself changed.
            s.setRefreshNonce((n) => n + 1);
            const current = s.selectedFilename();
            // A deleted file closes its view — unless it holds unsaved
            // edits, which stay open under a "deleted" banner.
            if (current && !res.files.some((f) => f.filename === current) && !untrack(s.detailDirty)) {
                s.setSelectedFilename("");
            }
        })
        .catch((e: Error) => {
            if (requestId !== s.latestRequestId) return;
            s.setFilesError(`Failed to list memory files: ${e.message ?? e}`);
        })
        .finally(() => {
            if (requestId !== s.latestRequestId) return;
            s.setFilesLoading(false);
        });
}

// Reactive grid + detail updates: subscribe to agent:memory:changed for
// every agent currently in the grid, re-subscribing whenever the agent
// SET changes (same agentIdsKey dependency the count-fetch effect
// (watchAgentSet) uses, for the same reason — a brand-new-but-equivalent agents() array
// must not tear down and rebuild every subscription).
// SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md.
//
// Debounced per agent id (~250ms): a burst of rapid writes to the same
// agent (e.g. a script issuing several MemoryWrite calls in a loop)
// coalesces into one refetch instead of one RPC round-trip per write.
// Keyed per agent id so a burst on one agent never delays another's.
//
// Held on the state (component-instance lifetime, not
// per-effect-run) and pruned rather than blanket-cleared on each
// re-run — see the ReAgent P1 fix note below.
function watchMemoryChanges(s: ManagerState): void {
    createEffect(() => {
        s.agentIdsKey();
        const list = untrack(() => s.agents());
        const present = new Set(list.map((a) => a.id));

        // ReAgent P1 (PR #2932): only prune timers for agents that actually
        // LEFT the set. The previous version's onCleanup cleared the ENTIRE
        // map unconditionally on every re-run of this effect — which fires
        // on any agentIdsKey() change, i.e. any agent create/edit/delete
        // ANYWHERE in the app (same over-broad trigger the count-fetch
        // effect above already had to guard against). A debounced refetch
        // already scheduled for an agent that's still present got silently
        // canceled and never fired if an unrelated agent changed elsewhere
        // within the 250ms window — that agent's count/detail view then
        // stayed stale until its own next unrelated event.
        for (const [id, timer] of s.debounceTimers) {
            if (!present.has(id)) {
                clearTimeout(timer);
                s.debounceTimers.delete(id);
            }
        }

        const unsubs = list.map((agent) =>
            onMuxEvent(`agent:memory:changed:${agent.id}`, () => {
                const existing = s.debounceTimers.get(agent.id);
                if (existing !== undefined) clearTimeout(existing);
                s.debounceTimers.set(
                    agent.id,
                    setTimeout(() => {
                        s.debounceTimers.delete(agent.id);
                        fetchCountFor(s, agent);
                        if (untrack(s.selectedAgent)?.id === agent.id) {
                            refetchSelectedAgentFiles(s, agent);
                        }
                    }, 250)
                );
            })
        );
        const unsub = () => unsubs.forEach((u) => u());

        // Per-run cleanup: only the subscriptions this run created. Pending
        // debounce timers are this effect's OWN concern to prune above, not
        // this callback's — they must survive an effect re-run so a
        // still-present agent's scheduled refetch isn't lost.
        onCleanup(unsub);
    });

    // True component unmount only (registered at the component body's own
    // scope, not nested inside createEffect, so it does not re-fire on
    // every effect re-run the way the per-run onCleanup above does).
    onCleanup(() => {
        for (const timer of s.debounceTimers.values()) clearTimeout(timer);
        s.debounceTimers.clear();
    });
}

function launchOf(s: ManagerState, agent: AgentDefinition) {
    const cost = s.launchCosts()[agent.id];
    if (!cost) return null;
    return {
        total: deliveredTotal(cost),
        window: windowFor(s, agent.id),
        personalDelivered: cost.parts.some((p) => p.kind === "personal"),
    };
}

function renderAgentGrid(s: ManagerState): JSX.Element {
    return (
        <div class="native-memory-manager-grid-view">
            <p class="native-memory-manager-hint">
                Each agent's own native memory — what it has chosen to remember. Select an agent to browse
                its files, versions and diffs.
            </p>
            <Show
                when={s.agents().length > 0}
                fallback={
                    <div class="native-memory-manager-empty">
                        {s.agentsLoading() ? "Loading agents…" : "No agents defined yet."}
                    </div>
                }
            >
                <MemoryAgentFilterBar
                    value={s.nameFilter}
                    onInput={s.setNameFilter}
                    onClear={() => s.setNameFilter("")}
                    onlyWithMemories={s.onlyWithMemories}
                    onOnlyWithMemoriesChange={s.setOnlyWithMemories}
                    sortBy={s.sortBy}
                    onSortChange={(sort) => setSortBy(s, sort)}
                />
                <MemoryFleetSummary agents={s.fleetAgents()} />
                <Show
                    when={s.visibleAgents().length > 0}
                    fallback={
                        <div class="native-memory-manager-empty">
                            {/* Distinct from "No agents defined yet." above — that
                                means zero s.agents exist at all; this means the fetch
                                succeeded but the current filter(s) matched nothing. */}
                            {s.nameFilter()
                                ? `No agents match "${s.nameFilter()}"`
                                : "No agents match the current filter."}
                        </div>
                    }
                >
                    <div class="native-memory-manager-grid">
                        <For each={s.visibleAgents()}>
                            {(agent) => (
                                <MemoryAgentCard
                                    agent={agent}
                                    count={s.counts()[agent.id] ?? { kind: "loading" }}
                                    onSelect={(agent) => setSelectedAgent(s, agent)}
                                launch={launchOf(s, agent)}
                                />
                            )}
                        </For>
                    </div>
                </Show>
            </Show>
        </div>
    );
}

function renderDetail(s: ManagerState, agent: Accessor<AgentDefinition>): JSX.Element {
    return (
        <div class="native-memory-manager-detail">
            {/* One header serves both detail screens (file grid, then
                version history). The back button walks the drill-down
                back exactly ONE level rather than always jumping to the
                agent grid, so the file grid is a real stop on the path
                and not a screen you can only leave by starting over. */}
            {renderDetailHeader(s, agent)}

            <Show when={s.filesError()}>
                <div class="native-memory-manager-error">{s.filesError()}</div>
            </Show>

            <Show when={s.selectedFilename()} fallback={renderFileGrid(s, agent)}>
                <div class="native-memory-manager-body native-memory-manager-body--pinned">
                    {/* Keyed on agentId:filename only — a clean remount
                        when switching agent/file. NOT on s.refreshNonce any
                        more (it was, Codex P1 PR #2932): a remount per live
                        change would wipe an open draft. The view reloads in
                        place on each s.refreshNonce bump instead. */}
                    <Show when={`${agent().id}:${s.selectedFilename()}`} keyed>
                        {(_key) => (
                            <NativeMemoryFileView
                                agentId={agent().id}
                                filename={s.selectedFilename()}
                                refreshNonce={s.refreshNonce}
                                onDirtyChange={s.setDetailDirty}
                            />
                        )}
                    </Show>
                </div>
            </Show>
        </div>
    );
}

function renderDetailHeader(s: ManagerState, agent: Accessor<AgentDefinition>): JSX.Element {
    return (
        <div class="native-memory-manager-header">
            <Button
                class="native-memory-manager-back"
                density="compact"
                onClick={() => {
                    if (!s.selectedFilename()) {
                        setSelectedAgent(s, null);
                        return;
                    }
                    if (s.detailDirty() && !window.confirm("Discard your unsaved changes?")) return;
                    s.setSelectedFilename("");
                }}
            >
                {s.selectedFilename() ? "← All files" : "← All agents"}
            </Button>
            <span class="native-memory-manager-crumbs">
                <span class="native-memory-manager-agent-name">{agentLabel(agent())}</span>
                <Show when={s.unverified()}>
                    <span
                        class="native-memory-manager-unverified"
                        {...tip("Found from the agent's settings, not from its own launch. Read-only until the agent launches.")}
                    >
                        Unverified folder
                    </span>
                </Show>
                <Show when={s.selectedFilename()}>
                    {(filename) => (
                        <>
                            <span class="native-memory-manager-crumb-sep" aria-hidden="true">
                                {"·"}
                            </span>
                            <span class="native-memory-manager-filename">{filename()}</span>
                        </>
                    )}
                </Show>
            </span>
        </div>
    );
}

function renderFileGrid(s: ManagerState, agent: Accessor<AgentDefinition>): JSX.Element {
    return (
        <div class="native-memory-manager-file-grid-view">
            <MemoryAdoptionPanel agentId={agent().id} agentName={agentLabel(agent())} />
            <Show when={s.launchCosts()[agent().id]}>
                {(cost) => <AgentLaunchCost cost={cost()} window={windowFor(s, agent().id)} />}
            </Show>
            <MemoryClaimsPanel agentId={agent().id} agentName={agentLabel(agent())} />
            <NativeMemoryCreate
                agentId={agent().id}
                filenames={s.files().map((f) => f.filename)}
                hasIndex={s.filesLoading() || s.filesError() !== null || s.files().some((f) => f.is_index)}
                readOnly={s.unverified()}
                listUnknown={s.filesLoading() || s.filesError() !== null}
                onCreated={(filename, createdFor) => {
                    // The write may finish after the person moved to
                    // another agent: its file isn't theirs to open.
                    if (s.selectedAgent()?.id !== createdFor) return;
                    refetchSelectedAgentFiles(s, agent());
                    // Only from the grid: a file opened meanwhile, with edits
                    // in it, stays open.
                    if (s.selectedFilename() === "") s.setSelectedFilename(filename);
                }}
            />
            {/* Distinct from the "no files" empty state below:
                an unresolved list must not read as an agent that
                has remembered nothing, the same four-state care
                MemoryAgentCard takes with its own count. */}
            <Show
                when={!s.filesLoading()}
                fallback={<div class="native-memory-manager-empty">Loading files…</div>}
            >
                <Show
                    when={s.sortedFiles().length > 0}
                    fallback={
                        <div class="native-memory-manager-empty">
                            {s.filesError()
                                ? "This agent's memory directory could not be read."
                                : "This agent hasn't remembered anything yet."}
                        </div>
                    }
                >
                    <Show when={s.sortedFiles().length >= MIN_FILES_FOR_DISTRIBUTION}>
                        <div class="memory-file-distribution" data-testid="memory-file-distribution">
                            {fileDistributionLabel(fileDistribution(s.sortedFiles().map((f) => f.tokens_est ?? 0))!)}
                        </div>
                    </Show>
                    <div class="native-memory-manager-file-grid">
                        <For each={s.sortedFiles()}>
                            {(file) => (
                                <MemoryFileCard
                                    file={file}
                                    onSelect={s.setSelectedFilename}
                                    claude={resolveProviderAlias(s.selectedAgent()?.provider ?? "") === "claude"}
                                />
                            )}
                        </For>
                    </div>
                </Show>
            </Show>
        </div>
    );
}

NativeMemoryManager.displayName = "NativeMemoryManager";
