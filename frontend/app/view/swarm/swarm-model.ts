// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { callBackendService } from "@/store/wos";
import { createSignal, type Accessor, type Setter } from "solid-js";

// ── Types ────────────────────────────────────────────────────────────────

export interface AgentOverview {
    agent_id: string;
    name: string;
    status: "active" | "idle" | "offline";
    session_count: number;
    total_tokens: number;
    last_active_at: number;
}

export interface ActiveSubagent {
    agent_id: string;
    slug: string;
    parent_agent: string;
    session_id: string;
    status: "active" | "completed";
    last_event_at: number;
    event_count: number;
    model: string | null;
}

export interface SearchResult {
    agent_id: string;
    session_id: string;
    timestamp: number;
    event_type: string;
    preview: string;
    score: number;
}

export interface HistorySessionMeta {
    session_id: string;
    file_path: string;
    provider: string;
    model: string;
    slug: string;
    working_directory: string;
    created_at: number;
    modified_at: number;
    message_count: number;
    first_user_message: string;
    file_size_bytes: number;
    git_branch: string;
    total_tokens: number;
    subagent_count: number;
}

export interface HistoryMessage {
    role: string;
    content: string;
    timestamp: number;
    tool_uses: { name: string; argument_summary: string }[];
}

export interface HistorySession {
    meta: HistorySessionMeta;
    messages: HistoryMessage[];
}

export type SwarmTab = "active" | "retired";

/**
 * The pane has two faces: the list (front) and entry detail (back).
 * `selectedEntry` non-null flips the pane to the detail face.
 * Today only subagent entries exist (agent listing is TODO — see
 * docs/specs/swarm-redesign-active-retired-2026-05-03.md).
 */
export type SwarmEntry = { kind: "subagent"; data: ActiveSubagent };

// ── Activity tab types ──────────────────────────────────────────────────
// Mirrors `backend::process_tracker::TrackedProcess` + RPC responses
// `AgentProcessListResult` / `AgentTrackedBlocksResult`.

/**
 * Tracking confidence for a block's process tracker. `"high"` on
 * Windows (Job Objects) and Linux (cgroups v2); `"best_effort"` on
 * macOS (process groups — escape via `setsid` or launchd is possible);
 * `"none"` when no tracker is registered.
 */
export type TrackingConfidence = "high" | "best_effort" | "none";

export interface AgentProcessInfo {
    pid: number;
    command: string;
    rss_bytes: number;
    started_at_ms: number;
}

/**
 * Per-block tracker snapshot. Keyed by block_id in the UI. The
 * swarm Activity tab groups agents by block and renders their
 * processes below each group header.
 */
export interface AgentProcessGroup {
    block_id: string;
    /** Display name for the agent (resolved from block meta at render time). */
    agent_name?: string;
    confidence: TrackingConfidence;
    processes: AgentProcessInfo[];
}

// ── ViewModel ────────────────────────────────────────────────────────────

export class SwarmViewModel implements ViewModel {
    viewType = "swarm";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "diagram-project";
    viewName: Accessor<string> = () => "Swarm";
    noPadding: Accessor<boolean> = () => true;

    get viewComponent(): ViewComponent {
        return null; // set by barrel
    }

    // Selected tab (Active / Retired)
    private _tab = createSignal<SwarmTab>("active");
    tabAtom: Accessor<SwarmTab> = this._tab[0];
    setTab: Setter<SwarmTab> = this._tab[1];

    // Detail-face entry. Non-null = pane is flipped to detail.
    private _selectedEntry = createSignal<SwarmEntry | null>(null);
    selectedEntryAtom: Accessor<SwarmEntry | null> = this._selectedEntry[0];
    setSelectedEntry: Setter<SwarmEntry | null> = this._selectedEntry[1];

    // Agent overview list
    private _agents = createSignal<AgentOverview[]>([]);
    agentsAtom: Accessor<AgentOverview[]> = this._agents[0];
    private setAgents: Setter<AgentOverview[]> = this._agents[1];

    // Active subagents
    private _subagents = createSignal<ActiveSubagent[]>([]);
    subagentsAtom: Accessor<ActiveSubagent[]> = this._subagents[0];
    private setSubagents: Setter<ActiveSubagent[]> = this._subagents[1];

    // Activity tab — per-block process-tracker groups. Populated by the
    // swarm-side `refreshActivity()` call plus delta events.
    private _activity = createSignal<AgentProcessGroup[]>([]);
    activityAtom: Accessor<AgentProcessGroup[]> = this._activity[0];
    private setActivity: Setter<AgentProcessGroup[]> = this._activity[1];

    // Instances tab — every AgentInstance ever created on this tab.
    // Surfaces a "where's that agent I started earlier?" answer per
    // SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23 PR G.
    private _instances = createSignal<AgentInstance[]>([]);
    instancesAtom: Accessor<AgentInstance[]> = this._instances[0];
    private setInstances: Setter<AgentInstance[]> = this._instances[1];

    // Search
    private _searchQuery = createSignal<string>("");
    searchQueryAtom: Accessor<string> = this._searchQuery[0];
    setSearchQuery: Setter<string> = this._searchQuery[1];

    private _searchResults = createSignal<SearchResult[]>([]);
    searchResultsAtom: Accessor<SearchResult[]> = this._searchResults[0];
    private setSearchResults: Setter<SearchResult[]> = this._searchResults[1];

    private _searching = createSignal<boolean>(false);
    searchingAtom: Accessor<boolean> = this._searching[0];
    private setSearching: Setter<boolean> = this._searching[1];

    // History
    private _historySessions = createSignal<HistorySessionMeta[]>([]);
    historySessionsAtom: Accessor<HistorySessionMeta[]> = this._historySessions[0];
    private setHistorySessions: Setter<HistorySessionMeta[]> = this._historySessions[1];

    private _historyTotal = createSignal<number>(0);
    historyTotalAtom: Accessor<number> = this._historyTotal[0];
    private setHistoryTotal: Setter<number> = this._historyTotal[1];

    private _historyHasMore = createSignal<boolean>(false);
    historyHasMoreAtom: Accessor<boolean> = this._historyHasMore[0];
    private setHistoryHasMore: Setter<boolean> = this._historyHasMore[1];

    private _historyLoading = createSignal<boolean>(false);
    historyLoadingAtom: Accessor<boolean> = this._historyLoading[0];
    private setHistoryLoading: Setter<boolean> = this._historyLoading[1];

    private _historyLoaded = false;

    private _selectedSession = createSignal<HistorySession | null>(null);
    selectedSessionAtom: Accessor<HistorySession | null> = this._selectedSession[0];
    private setSelectedSession: Setter<HistorySession | null> = this._selectedSession[1];

    private _sessionLoading = createSignal<boolean>(false);
    sessionLoadingAtom: Accessor<boolean> = this._sessionLoading[0];
    private setSessionLoading: Setter<boolean> = this._sessionLoading[1];

    // Loading
    private _loading = createSignal<boolean>(true);
    loadingAtom: Accessor<boolean> = this._loading[0];
    private setLoading: Setter<boolean> = this._loading[1];

    // Event subscriptions
    private unsubs: (() => void)[] = [];

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;

        // Load initial data
        this.loadOverview();

        // Subscribe to subagent lifecycle events
        const unsubSpawned = waveEventSubscribe({
            eventType: "subagent:spawned",
            handler: () => this.loadSubagents(),
        });
        if (unsubSpawned) this.unsubs.push(unsubSpawned);

        const unsubCompleted = waveEventSubscribe({
            eventType: "subagent:completed",
            handler: () => this.loadSubagents(),
        });
        if (unsubCompleted) this.unsubs.push(unsubCompleted);

        // Process tracker deltas — refresh the Activity tab on every
        // add/exit. Cheap (one RPC returning a small JSON), debouncing
        // isn't needed since events fire on a ~2s cadence from the
        // backend poller.
        const unsubProcAdded = waveEventSubscribe({
            eventType: "agent:process-added",
            handler: () => this.refreshActivity(),
        });
        if (unsubProcAdded) this.unsubs.push(unsubProcAdded);
        const unsubProcExited = waveEventSubscribe({
            eventType: "agent:process-exited",
            handler: () => this.refreshActivity(),
        });
        if (unsubProcExited) this.unsubs.push(unsubProcExited);

        // Seed the Activity tab on mount so the list is populated
        // before the user clicks the tab (avoids empty-on-open flash).
        this.refreshActivity();
    }

    /**
     * Refresh the Instances tab — list every AgentInstance row in
     * the database. Sorted newest-first since the most recent runs
     * are the most likely target for "open it again". Backend
     * sorts at the SQL layer.
     */
    refreshInstances = async (): Promise<void> => {
        try {
            const list = await RpcApi.ListAgentInstancesCommand(TabRpcClient, {});
            this.setInstances(list ?? []);
        } catch {
            // Silent fail — older backends without the RPC.
        }
    };

    /**
     * Refresh the Activity tab: list every tracked block, then fetch
     * each block's process list + confidence level, then set the atom.
     * Runs serially — block counts are small (a few agents at most).
     */
    refreshActivity = async (): Promise<void> => {
        try {
            const { block_ids } = await RpcApi.AgentTrackedBlocksCommand(TabRpcClient, {});
            const groups: AgentProcessGroup[] = await Promise.all(
                block_ids.map(async (block_id) => {
                    try {
                        const res = await RpcApi.AgentProcessListCommand(TabRpcClient, {
                            block_id,
                        });
                        return {
                            block_id: res.block_id,
                            confidence: res.confidence,
                            processes: res.processes,
                        };
                    } catch {
                        return {
                            block_id,
                            confidence: "none" as TrackingConfidence,
                            processes: [],
                        };
                    }
                }),
            );
            this.setActivity(groups);
        } catch {
            // Silent fail — tracker global may not be initialized (tests),
            // or backend may not have the new RPC yet (older version
            // running in dev). Empty list is a safe default.
        }
    };

    loadOverview = async (): Promise<void> => {
        this.setLoading(true);
        try {
            await Promise.all([this.loadSubagents()]);
        } finally {
            this.setLoading(false);
        }
    };

    loadSubagents = async (): Promise<void> => {
        try {
            const result = await callBackendService("subagent", "ListActive", []);
            const list = (result as ActiveSubagent[]) ?? [];
            this.setSubagents(list);
        } catch {
            // silently ignore
        }
    };

    loadHistory = async (offset = 0): Promise<void> => {
        if (this._historyLoaded && offset === 0) return;
        this.setHistoryLoading(true);
        try {
            const result = await callBackendService("history", "List", [
                null, // provider filter
                null, // project filter
                offset,
                50,   // limit
                "modified_at",
                "desc",
            ]);
            const data = result as { sessions: HistorySessionMeta[]; total: number; has_more: boolean };
            if (offset === 0) {
                this.setHistorySessions(data.sessions ?? []);
            } else {
                this.setHistorySessions([...this.historySessionsAtom(), ...(data.sessions ?? [])]);
            }
            this.setHistoryTotal(data.total ?? 0);
            this.setHistoryHasMore(data.has_more ?? false);
            this._historyLoaded = true;
        } catch {
            // silently ignore
        } finally {
            this.setHistoryLoading(false);
        }
    };

    loadSession = async (sessionId: string): Promise<void> => {
        this.setSessionLoading(true);
        this.setSelectedSession(null);
        try {
            const result = await callBackendService("history", "Get", [sessionId]);
            const data = result as { session?: HistorySession; error?: string };
            if (data.session) {
                this.setSelectedSession(data.session);
            }
        } catch {
            // silently ignore
        } finally {
            this.setSessionLoading(false);
        }
    };

    closeSession = (): void => {
        this.setSelectedSession(null);
    };

    refreshHistory = async (): Promise<void> => {
        this._historyLoaded = false;
        await callBackendService("history", "Refresh", []);
        await this.loadHistory(0);
    };

    search = async (query: string): Promise<void> => {
        if (!query.trim()) {
            this.setSearchResults([]);
            return;
        }
        this.setSearching(true);
        try {
            const result = await callBackendService("subagent", "SearchHistory", [
                query,
                50,
            ]);
            this.setSearchResults((result as SearchResult[]) ?? []);
        } catch {
            this.setSearchResults([]);
        } finally {
            this.setSearching(false);
        }
    };

    dispose(): void {
        for (const unsub of this.unsubs) {
            unsub();
        }
        this.unsubs = [];
    }
}
