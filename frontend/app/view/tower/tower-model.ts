// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tower's state behind its native pane tab (tower.tsx): the latest sample,
// polled from srv (`tower.sample`) only while the pane is visible
// (SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md §6.2). With a connection it
// samples that machine instead, through AgentMux's helper there (§8.2).

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { remotesList } from "@/app/store/remotes-store";
import { RpcApi, type TowerPeerInfo, type TowerSnapshot } from "@/app/store/rpc-api";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import { TabRpcClient } from "@/app/store/rpc-util";
import { settingsAtom } from "@/store/global";
import { type Accessor, createEffect, createMemo, createSignal, on, onCleanup, type Setter } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import {
    type CpuMode,
    orderRail,
    type ProcessGrouping,
    railEntries,
    type RailSort,
    type Sort,
    type TowerView,
} from "./tower-util";

/** A sample with no CPU rates yet (the first one) is followed up this soon,
 *  instead of a whole interval of dashes. */
const FIRST_RATE_DELAY_MS = 1000;
const RETRY_DELAY_MS = 5000;
/** Another machine's first answer may wait on ssh asking the user something,
 *  or on installing the helper there. */
const REMOTE_TIMEOUT_MS = 180_000;
/** On another machine the filter is applied there: wait for typing to pause. */
const REMOTE_FILTER_DELAY_MS = 300;
/** The rail's sparklines: the last minute at the 2 s refresh. */
export const HISTORY_POINTS = 30;
/** A CPU-ordered rail orders by the average of this many recent samples, so
 *  a moment's spike doesn't reorder it. */
const SMOOTHING_POINTS = 5;
const RAIL_SORTS: readonly RailSort[] = ["pane", "cpu", "mem", "name"];

export class TowerViewModel {
    viewType = "tower";
    blockId: string;
    view: Accessor<TowerView>;
    /** The machine shown: `""` is this computer, else a connection name. */
    connection: Accessor<string>;
    /** Showing another machine: no command lines. */
    remote: Accessor<boolean>;
    /** Whether the machine shown has tasks: this computer and a paired
     *  AgentMux computer do; an SSH host or WSL distribution doesn't. */
    hasTasks: Accessor<boolean>;
    /** The view shown: a machine without tasks shows only its processes. */
    effectiveView: Accessor<TowerView>;
    /** The Agents view's selected rail entry (`RailEntry::id`), as chosen. */
    selected: Accessor<string>;
    /** The rail entry the Agents view shows: the chosen one, else the first. */
    shownEntry: Accessor<string>;
    /** The Processes view shows only this owner's processes (`RailEntry::id`,
     *  `ownerOf`), carried over from the Agents view; `""` for all. */
    only: Accessor<string>;
    railSort: Accessor<RailSort>;
    /** Each rail entry's recent CPU, oldest first (fraction of one core). */
    history: Accessor<ReadonlyMap<string, readonly number[]>>;
    private setHistory: Setter<ReadonlyMap<string, readonly number[]>>;
    remotes: Accessor<RemoteRecord[]>;
    /** AgentMux computers this one is paired with (`peer:<id>`). */
    peers: Accessor<TowerPeerInfo[]>;
    /** Whether this computer lets its paired devices see its processes. */
    sharing: Accessor<boolean>;
    cpuMode: Accessor<CpuMode>;
    snapshot: Accessor<TowerSnapshot | null>;
    error: Accessor<string | null>;
    sort: Accessor<Sort>;
    setSort: Setter<Sort>;
    expanded: Accessor<ReadonlySet<string>>;
    /** How the Processes view groups its list: by agent where the machine
     *  has agents, else by app as Task Manager does, or not at all. */
    grouping: Accessor<ProcessGrouping>;
    /** The Processes view has opened its busiest agents once, on its first
     *  list; after that what is open is the user's. */
    ownersOpened = false;
    filter: Accessor<string>;
    setFilter: Setter<string>;
    viewName: Accessor<string>;
    private setMeta: (patch: Record<string, unknown>) => void;
    private setSnapshot: (snap: TowerSnapshot | null) => void;
    private setError: Setter<string | null>;
    private setPeers: Setter<TowerPeerInfo[]>;
    private setExpanded: Setter<ReadonlySet<string>>;
    private timer: ReturnType<typeof setTimeout> | undefined;
    /** Bumped on every start and stop, so a reply from an older poll loop
     *  is dropped. */
    private generation = 0;
    /** The request in flight, settled or not: the next one waits for it. */
    private inFlight: Promise<void> = Promise.resolve();
    /** The machine `inFlight` asks (`""` for this computer). */
    private inFlightFor = "";
    private lastStart: [boolean, string, string] | undefined;
    /** The machine that failed and waits for Retry. */
    private stalledFor: string | null = null;
    private setStalled: Setter<boolean>;
    /** Another machine failed and polling stopped until `retry()`. */
    stalled: Accessor<boolean>;

    // Built by `create(ctx)` in the instance's own reactive root (host rule 8),
    // so the effects and cleanup below live and die with the pane tab.
    constructor(ctx: PaneTabHostContext) {
        this.blockId = ctx.blockId;
        this.setMeta = (patch) => void ctx.setMeta(patch);
        // "host" is what the Processes view was called until 2026-10-10.
        this.view = createMemo<TowerView>(() => {
            const v = ctx.meta()?.["tower:view"];
            return v === "processes" || v === "host" ? "processes" : "agents";
        });
        this.connection = createMemo(() => {
            const c = ctx.meta()?.["tower:connection"];
            return typeof c === "string" && c !== "local" ? c : "";
        });
        this.remote = () => this.connection() !== "";
        // Another AgentMux computer has panes, so it has tasks too; an SSH host
        // or WSL distribution has only processes.
        this.hasTasks = () => !this.remote() || this.connection().startsWith("peer:");
        this.effectiveView = createMemo<TowerView>(() => (this.hasTasks() ? this.view() : "processes"));
        this.selected = createMemo(() => {
            const v = ctx.meta()?.["tower:agent"];
            return typeof v === "string" ? v : "";
        });
        this.only = createMemo(() => {
            const v = ctx.meta()?.["tower:only"];
            return typeof v === "string" && this.hasTasks() ? v : "";
        });
        this.railSort = createMemo<RailSort>(() => {
            const v = ctx.meta()?.["tower:railsort"];
            return RAIL_SORTS.includes(v as RailSort) ? (v as RailSort) : "pane";
        });
        [this.history, this.setHistory] = createSignal<ReadonlyMap<string, readonly number[]>>(new Map());
        this.remotes = remotesList();
        [this.peers, this.setPeers] = createSignal<TowerPeerInfo[]>([]);
        void this.refreshPeers();
        this.sharing = () => settingsAtom()?.["tower:sharewithpaired"] === true;
        this.cpuMode = createMemo<CpuMode>(() => (ctx.meta()?.["tower:cpu"] === "core" ? "core" : "machine"));
        // Saved as "off" until 2026-10-10, when grouping was by app or nothing.
        this.grouping = createMemo<ProcessGrouping>(() => {
            const g = ctx.meta()?.["tower:group"];
            if (g === "off" || g === "none") return "none";
            if (g === "app") return "app";
            return this.hasTasks() ? "agent" : "app";
        });
        this.viewName = createMemo(() => {
            if (!this.remote()) return this.view() === "processes" ? "Tower · Processes" : "Tower";
            const peer = this.peers().find((p) => p.connection === this.connection());
            return `Tower · ${peer?.hostname || this.connection()}`;
        });
        // A store merged by `id`: a task or process still there keeps its
        // object across polls, so its row (an expanded command line, a text
        // selection) survives the refresh instead of being rebuilt.
        const [state, setState] = createStore<{ snap: TowerSnapshot | null }>({ snap: null });
        this.snapshot = () => state.snap;
        this.setSnapshot = (snap) => {
            if (!snap) {
                this.setHistory(new Map());
                return setState("snap", null);
            }
            this.recordHistory(snap);
            setState("snap", reconcile(snap, { key: "id", merge: true }));
        };
        this.shownEntry = createMemo(() => {
            const snap = this.snapshot();
            if (!snap) return this.selected();
            const ids = orderRail(railEntries(snap), this.railSort(), this.smoothedCpu).map((e) => e.id);
            return ids.includes(this.selected()) ? this.selected() : (ids[0] ?? "");
        });
        [this.error, this.setError] = createSignal<string | null>(null);
        [this.stalled, this.setStalled] = createSignal(false);
        [this.sort, this.setSort] = createSignal<Sort>({ key: "cpu", desc: true });
        [this.expanded, this.setExpanded] = createSignal<ReadonlySet<string>>(new Set<string>());
        [this.filter, this.setFilter] = createSignal("");

        // Another machine filters there, so its filter restarts the polling
        // once typing pauses; this computer's list is filtered in the pane.
        const [remoteFilter, setRemoteFilter] = createSignal("");
        createEffect(
            on([this.filter, this.remote], ([q, remote]) => {
                if (!remote) return setRemoteFilter("");
                const t = setTimeout(() => setRemoteFilter(q.trim()), REMOTE_FILTER_DELAY_MS);
                onCleanup(() => clearTimeout(t));
            })
        );

        // Poll while visible; switching view, machine or remote filter
        // restarts at once with the right request.
        createEffect(
            on(
                [() => ctx.visibility() === "active", this.effectiveView, this.connection, remoteFilter],
                ([visible, view, connection, filter]) => {
                    this.stop();
                    // A machine that failed stays stopped until Retry, whatever
                    // else changes (visibility, filter): each try could reopen
                    // ssh and ask about installing again.
                    if (connection && this.stalledFor === connection) return;
                    this.stalledFor = null;
                    if (visible) this.start(view === "processes", connection, filter);
                }
            )
        );
        // Another machine's numbers never show under this one's name.
        createEffect(on(this.connection, () => this.setSnapshot(null), { defer: true }));
        onCleanup(() => this.stop());
    }

    /** Switch views. Going from the Agents view to Processes keeps the agent
     *  shown there as a filter (`only`), as Lens keeps a namespace; `only`
     *  names another owner instead. */
    setView(view: TowerView, only?: string): void {
        // Back on the Agents view the filter is dropped; going to Processes
        // again carries whichever agent is shown then.
        const carried =
            view === "processes" && this.effectiveView() === "agents"
                ? (only ?? this.shownEntry())
                : view === "agents" && this.only()
                  ? ""
                  : undefined;
        this.setMeta({
            "tower:view": view === "processes" ? "processes" : null,
            ...(carried !== undefined ? { "tower:only": carried || null } : {}),
        });
    }

    /** The Processes view shows every process again. */
    showAll(): void {
        this.setMeta({ "tower:only": null });
    }

    /** Select a rail entry in the Agents view. */
    select(id: string): void {
        this.setMeta({ "tower:agent": id || null });
    }

    setRailSort(sort: RailSort): void {
        this.setMeta({ "tower:railsort": sort === "pane" ? null : sort });
    }

    /** A rail entry's recent average CPU, for a CPU-ordered rail. */
    smoothedCpu = (id: string): number | undefined => {
        const points = this.history().get(id);
        if (!points?.length) return undefined;
        const recent = points.slice(-SMOOTHING_POINTS);
        return recent.reduce((a, b) => a + b, 0) / recent.length;
    };

    /** Append each rail entry's CPU to its history; an entry gone from the
     *  rail loses its history. */
    private recordHistory(snap: TowerSnapshot): void {
        const prev = this.history();
        const next = new Map<string, readonly number[]>();
        for (const e of railEntries(snap)) {
            const points = prev.get(e.id) ?? [];
            next.set(e.id, e.cpu == null ? points : [...points, e.cpu].slice(-HISTORY_POINTS));
        }
        this.setHistory(next);
    }

    /** `""` for this computer. */
    setConnection(connection: string): void {
        // An owner named on one machine means nothing on another.
        this.setMeta({ "tower:connection": connection || null, ...(this.only() ? { "tower:only": null } : {}) });
    }

    async refreshPeers(): Promise<void> {
        try {
            this.setPeers((await RpcApi.TowerPeersCommand(TabRpcClient, { timeout: 5000 })).peers);
        } catch {
            // Keep the last list.
        }
    }

    /** Pair with another AgentMux computer from its link and show it. Throws
     *  the reason it couldn't. */
    async pair(link: string): Promise<void> {
        const peer = await RpcApi.TowerPairCommand(TabRpcClient, { link: link.trim() }, { timeout: 20000 });
        await this.refreshPeers();
        this.setConnection(peer.connection);
    }

    /** Forget a paired computer; a failure (the keychain refused, say) shows
     *  in the pane's error line, and the computer stays paired. */
    async forget(connection: string): Promise<void> {
        try {
            const r = await RpcApi.TowerForgetCommand(TabRpcClient, { connection }, { timeout: 30000 });
            this.setPeers(r.peers);
            if (this.connection() === connection) this.setConnection("");
        } catch (e) {
            this.setError(e instanceof Error ? e.message : String(e));
        }
    }

    setSharing(on: boolean): void {
        void RpcApi.SetConfigCommand(TabRpcClient, {
            "tower:sharewithpaired": on ? true : null,
        } as unknown as SettingsType);
    }

    setGrouping(grouping: ProcessGrouping): void {
        this.setMeta({ "tower:group": grouping === "agent" ? null : grouping });
    }

    /** Open these Processes groups (`owner:<key>` keys). */
    open(keys: string[]): void {
        const next = new Set(this.expanded());
        for (const k of keys) next.add(k);
        this.setExpanded(next);
    }

    setCpuMode(mode: CpuMode): void {
        this.setMeta({ "tower:cpu": mode === "core" ? "core" : null });
    }

    /** Collapse these tree nodes (`fold:<id>` keys). */
    fold(keys: string[]): void {
        const next = new Set(this.expanded());
        for (const k of keys) next.add(k);
        this.setExpanded(next);
    }

    /** Expand these tree nodes again. */
    unfold(keys: string[]): void {
        const next = new Set(this.expanded());
        for (const k of keys) next.delete(k);
        this.setExpanded(next);
    }

    toggleExpanded(taskId: string): void {
        const next = new Set(this.expanded());
        if (!next.delete(taskId)) next.add(taskId);
        this.setExpanded(next);
    }

    /** The task a process-list row belongs to, by id. */
    taskLabel = (taskId: string): string | undefined => this.snapshot()?.tasks.find((t) => t.id === taskId)?.label;

    dispose(): void {
        this.stop();
    }

    /** After another machine failed: ask again (the polling stopped there). */
    retry(): void {
        const last = this.lastStart;
        if (!last) return;
        this.stalledFor = null;
        this.stop();
        this.start(...last);
    }

    private start(host: boolean, connection: string, filter: string): void {
        const gen = ++this.generation;
        this.lastStart = [host, connection, filter];
        this.setStalled(false);
        // At most one request in flight: a new one (another filter, say)
        // waits for the last to answer. Another machine's helper measures CPU
        // since its previous request, so two back to back would read noise.
        // Only the same machine waits: another one's request (perhaps ssh
        // waiting on a password) mustn't hold up this one.
        const wait = this.inFlightFor === connection ? this.inFlight : Promise.resolve();
        void wait.then(() => {
            if (gen === this.generation) this.poll(gen, host, connection, filter);
        });
    }

    private stop(): void {
        this.generation++;
        if (this.timer !== undefined) clearTimeout(this.timer);
        this.timer = undefined;
    }

    private poll(gen: number, host: boolean, connection: string, filter: string): void {
        const request = connection
            ? RpcApi.TowerSampleCommand(
                  TabRpcClient,
                  { host: host || !connection.startsWith("peer:"), connection, filter, block_id: this.blockId },
                  { timeout: REMOTE_TIMEOUT_MS }
              )
            : RpcApi.TowerSampleCommand(TabRpcClient, { host });
        this.inFlightFor = connection;
        this.inFlight = request.then(
            () => undefined,
            () => undefined
        );
        const again = (delay: number) => {
            if (gen === this.generation) {
                this.timer = setTimeout(() => this.poll(gen, host, connection, filter), delay);
            }
        };
        request.then(
            (snap) => {
                if (gen !== this.generation) return;
                this.setSnapshot(snap);
                this.setError(null);
                again(hasNoRatesYet(snap) ? FIRST_RATE_DELAY_MS : snap.interval_ms);
            },
            (e) => {
                if (gen !== this.generation) return;
                this.setError(e instanceof Error ? e.message : String(e));
                // Another machine that failed (unreachable, no helper for its
                // platform, an install the user declined) is not asked again
                // on its own: each try could open ssh and ask about installing.
                if (connection) {
                    this.stalledFor = connection;
                    this.setStalled(true);
                } else again(RETRY_DELAY_MS);
            }
        );
    }
}

/** The first answer: everything it lists has no CPU rate yet. */
function hasNoRatesYet(snap: TowerSnapshot): boolean {
    const rows = snap.remote ? (snap.host?.processes ?? []) : snap.tasks;
    return rows.length > 0 && rows.every((r) => r.cpu == null);
}
