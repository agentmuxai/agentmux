// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// DroneViewModel — owns the per-pane state for the Drone widget.
//
// Phase 1.5 PR 4 (`docs/specs/SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md`
// §6 row 4) routes per-run state through the `drone-run-state` slice
// (#10) — same lifecycle pattern as slice #9 (browser-pane-state).
//
// What's reducer-backed (slot store, `recordDispatch` audit ring):
//   - activeRunId, status, blockResults, run output, run error
//
// What stays as view-model state (pure UI editing, no event-folded):
//   - draft graph, selection, running button-flag, runs list, errors

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    dispatch as dispatchDroneRun,
    registerPane,
    unregisterPane,
    type AgentBlockResult,
    type DroneRunStatus,
} from "@/app/store/drone-run-state-store";
import { waveEventSubscribe } from "@/app/store/wps";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, createSignal, type Accessor } from "solid-js";
import { createStore, produce, reconcile, type SetStoreFunction } from "solid-js/store";

import { blockMeta } from "./block-registry";
import {
    defaultViewport,
    emptyGraph,
    type BlockKind,
    type FlowEdge,
    type FlowNode,
    type DroneDefinition,
    type DroneGraph,
    type DroneRun,
    type DroneViewport,
} from "./drone-types";

const BLANK_DRONE = (): DroneDefinition => ({
    id: "",
    name: "Untitled Drone",
    description: "",
    graph: emptyGraph(),
    viewport: defaultViewport(),
    created_at: 0,
    updated_at: 0,
});

export class DroneViewModel implements ViewModel {
    viewType = "drone";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "diagram-project";
    viewName: Accessor<string>;
    viewText: Accessor<string | HeaderElem[]> = () => "";
    noPadding: Accessor<boolean> = () => true;

    get viewComponent(): ViewComponent {
        return null; // overridden by the barrel via Object.defineProperty
    }

    blockAtom: Accessor<Block | undefined>;

    // --- list of saved drones
    private _list = createSignal<DroneDefinition[]>([]);
    listAtom: Accessor<DroneDefinition[]> = this._list[0];
    setList = this._list[1];

    // --- the drone currently open in the canvas (a draft until saved)
    //
    // Backed by a Solid STORE, not a signal. A signal holding the whole
    // DroneDefinition replaces the object on every edit, so any reader
    // of `graph.nodes` re-runs — fatal for smooth node dragging at 50+
    // nodes. A store proxies each node/field independently, so moving
    // one node touches only that node's `position` binding. See
    // SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05.md §8.
    private _draftStore = createStore<DroneDefinition>(BLANK_DRONE());
    private draft: DroneDefinition = this._draftStore[0];
    private setDraftStore: SetStoreFunction<DroneDefinition> = this._draftStore[1];

    // Stable accessor — the rest of the codebase keeps calling
    // `m.draftAtom()`. Returns the live store proxy; property reads on
    // it (`.graph.nodes`) stay fine-grained inside a tracking scope.
    draftAtom: Accessor<DroneDefinition> = () => this.draft;

    /** Replace the entire draft (load / new / post-save normalize).
     *  Uses `reconcile` (keyed on node/edge `id`) so surviving DOM nodes
     *  and edges are diffed in place rather than torn down + rebuilt. */
    private setDraft(next: DroneDefinition): void {
        this.setDraftStore(reconcile(next));
    }

    // --- which node is selected in the canvas (for the InspectorPanel)
    private _selected = createSignal<string | null>(null);
    selectedAtom: Accessor<string | null> = this._selected[0];
    setSelected = this._selected[1];

    // --- canvas viewport (pan + zoom). Deliberately SEPARATE from the
    // draft store: panning/zooming updates only this one transform, so
    // it never re-runs a node binding. Synced into draft.viewport at
    // save time (so it persists). See spec §8.2.
    private _viewport = createSignal<DroneViewport>(defaultViewport());
    viewportAtom: Accessor<DroneViewport> = this._viewport[0];
    setViewport = (patch: Partial<DroneViewport>): void => {
        this._viewport[1]((v) => ({ ...v, ...patch }));
    };

    // --- canvas pixel size, kept current by the Canvas via a
    // ResizeObserver. Used to place a node at the visible center when a
    // top-bar chip is clicked (the no-drag fallback).
    private _canvasSize = createSignal<{ w: number; h: number }>({ w: 0, h: 0 });
    setCanvasSize = (size: { w: number; h: number }): void => {
        this._canvasSize[1](size);
    };

    // --- which EDGE is selected (for delete). Mutually exclusive with
    // node selection.
    private _selectedEdge = createSignal<string | null>(null);
    selectedEdgeAtom: Accessor<string | null> = this._selectedEdge[0];
    setSelectedEdge = (id: string | null): void => {
        this._selectedEdge[1](id);
        if (id) this._selected[1](null);
    };

    // --- run state
    //
    // `_running` is the in-flight `await RunDroneCommand` flag — bound
    // to the NodeTypeBar's Run button disable. It's a UI thing, separate from the
    // slot's `status` (which is "idle"|"running"|"done"|"failed", folded
    // from `dronerun:<id>` events). Keeping it in the view.
    private _running = createSignal<boolean>(false);
    runningAtom: Accessor<boolean> = this._running[0];
    setRunning = this._running[1];

    // ── Slot-projected cells (reducer-backed via slice #10) ────────
    //
    // These signals are written ONLY by the slot's projector. The
    // view dispatches `RunStarted` / `BlockDone` / etc. and the slot
    // calls back into the projector setters below to keep these in
    // sync. Reads stay through the accessors so the rest of the
    // codebase doesn't notice the migration.
    private _activeRunId = createSignal<string | null>(null);
    activeRunIdAtom: Accessor<string | null> = this._activeRunId[0];

    private _status = createSignal<DroneRunStatus>("idle");
    statusAtom: Accessor<DroneRunStatus> = this._status[0];

    private _blockResults = createSignal<Record<string, AgentBlockResult>>({});
    blockResultsAtom: Accessor<Record<string, AgentBlockResult>> = this._blockResults[0];

    blockResultAtom(blockId: string): AgentBlockResult | undefined {
        return this.blockResultsAtom()[blockId];
    }

    // ── Non-reducer-backed view-only cells ─────────────────────────
    private _runs = createSignal<DroneRun[]>([]);
    runsAtom: Accessor<DroneRun[]> = this._runs[0];
    setRuns = this._runs[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    // Active `dronerun:<id>` subscription. Stored so we can
    // unsubscribe on dispose / when the run changes.
    private activeRunUnsub: (() => void) | null = null;

    // Idempotency flag — block-cleanup paths can call dispose() more
    // than once on the same view model. A second call would re-fire
    // `dispatchDroneRun(..., Disposed)` against a slot the first
    // call already removed, throwing "unregistered pane" out of
    // teardown. Codex P2 on PR #844.
    private disposed = false;

    selectedNodeAtom: Accessor<FlowNode | null>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = getWaveObjectAtom(makeORef("block", blockId));
        this.viewName = createMemo(() => {
            const block = this.blockAtom();
            return (block?.meta?.["frame:title"] as string) ?? this.draftAtom().name;
        });
        this.selectedNodeAtom = createMemo(() => {
            const id = this.selectedAtom();
            if (!id) return null;
            return this.draftAtom().graph.nodes.find((n) => n.id === id) ?? null;
        });

        // Register the slot SYNCHRONOUSLY so the first dispatch (in
        // `run()`) never races against a missing pane.
        registerPane(this.blockId, {
            closed: () => {
                // Closed-flag transitions are handled via `dispose()`'s
                // unregisterPane — no view-model signal to mirror.
            },
            runId: (next) => this._activeRunId[1](next === "" ? null : next),
            droneId: () => {
                // Not mirrored — the view reads draft.id, not the slot's
                // droneId. Keeping the projection a no-op so the slot
                // surface stays uniform with slice #9.
            },
            status: (next) => this._status[1](next),
            blockResults: (next) => this._blockResults[1](next),
            output: () => {
                // The terminal `output` projection is unused today — the
                // Run panel reads the runs-list row instead, and the
                // inspector reads blockResults. Reserved for Phase 2
                // when a drone-level result panel lands.
            },
            error: () => {
                // Same rationale as `output` — terminal error surfaces
                // via the runs-list row. Reserved for Phase 2.
            },
        });

        void this.refreshList();
    }

    async refreshList(): Promise<void> {
        try {
            const list = await RpcApi.ListDronesCommand(TabRpcClient, {});
            this.setList(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load drones: ${(e as Error).message ?? e}`);
        }
    }

    /** Load a drone into the canvas as the draft. */
    async openDrone(id: string): Promise<void> {
        try {
            const wf = await RpcApi.GetDroneCommand(TabRpcClient, { id });
            if (wf) {
                this.setDraft(wf);
                this.setSelected(null);
                this.setViewport(wf.viewport ?? defaultViewport());
                this.dispatchIfAlive({ type: "Reset" }, "user");
                await this.refreshRuns(id);
            }
        } catch (e) {
            this.setError(`Failed to open drone: ${(e as Error).message ?? e}`);
        }
    }

    /** Start a fresh blank drone in the canvas. */
    newDrone(): void {
        this.setDraft(BLANK_DRONE());
        this.setSelected(null);
        this.setViewport(defaultViewport());
        this.setRuns([]);
        this.dispatchIfAlive({ type: "Reset" }, "user");
        if (this.activeRunUnsub) {
            this.activeRunUnsub();
            this.activeRunUnsub = null;
        }
    }

    /** Persist the current draft. Returns the saved id. */
    async save(): Promise<string | null> {
        // Fold the live canvas viewport into the draft store so it
        // persists with the graph.
        this.setDraftStore("viewport", { ...this.viewportAtom() });
        const wf = this.draftAtom();
        try {
            const saved = await RpcApi.UpsertDroneCommand(TabRpcClient, wf);
            this.setDraft(saved);
            await this.refreshList();
            return saved.id;
        } catch (e) {
            this.setError(`Save failed: ${(e as Error).message ?? e}`);
            return null;
        }
    }

    /** Save (if needed) then run. */
    async run(): Promise<void> {
        let id = this.draftAtom().id;
        if (!id) {
            const saved = await this.save();
            if (!saved) return;
            id = saved;
        } else {
            // Always save current canvas state before running.
            const ok = await this.save();
            if (!ok) return;
        }
        this.setRunning(true);
        // Reset folded run state BEFORE the RPC fires (reagent P2 on
        // #848): if the RPC throws, we still want the prior run's
        // blockResults / output / error cleared so the inspector
        // doesn't show stale results alongside the new error banner.
        this.dispatchIfAlive({ type: "Reset" }, "user");
        try {
            const r = await RpcApi.RunDroneCommand(TabRpcClient, { drone_id: id });
            // The dispose-during-await race: if `dispose()` ran while
            // we were awaiting the RPC, `this.disposed === true` now
            // and `dispatchIfAlive` no-ops below. `subscribeRun`
            // creates a WPS listener — gate it too so we don't leak
            // the subscription past dispose (reagent P1 on #848).
            if (this.disposed) return;
            this.dispatchIfAlive(
                { type: "RunStarted", runId: r.run_id, droneId: id },
                "user",
            );
            this.subscribeRun(r.run_id, id);
            // Backend inserts a `running` placeholder row synchronously
            // before this RPC resolves; the subscription above picks up
            // `RunDone` / `RunFailed` and re-refreshes the runs list
            // (Phase 1.5 PR 3, #830).
            await this.refreshRuns(id);
            if (this.disposed) return;
            // Race recovery: ultra-fast drones can finish + persist
            // their final row before we subscribe (codex P2 on #843).
            // Dispatched as `BackfilledFromRow`, the reducer treats
            // backfill as the authoritative final state.
            this.maybeBackfillFromTerminalRun(r.run_id, id);
        } catch (e) {
            this.setError(`Run failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setRunning(false);
        }
    }

    /** Subscribe to `dronerun:<runId>` WPS events for the active run.
     *  Replaces any prior subscription. The backend publishes one event
     *  per `RunEvent` variant — we route every event into the slot
     *  reducer and refresh the run row on terminal events. */
    private subscribeRun(runId: string, droneId: string): void {
        if (this.activeRunUnsub) {
            this.activeRunUnsub();
            this.activeRunUnsub = null;
        }
        this.activeRunUnsub = waveEventSubscribe({
            eventType: `dronerun:${runId}`,
            scope: "",
            handler: (event) => {
                const data = (event as { data?: RunEventWire }).data;
                if (!data) return;
                this.dispatchWireEvent(data);
                if (data.kind === "run_done" || data.kind === "run_failed") {
                    // Backend writes the row update BEFORE publishing
                    // the terminal event (codex P2 on #843), so a
                    // refresh here lands the final state.
                    void this.refreshRuns(droneId);
                    if (this.activeRunUnsub) {
                        this.activeRunUnsub();
                        this.activeRunUnsub = null;
                    }
                }
            },
        });
    }

    /** Translate a wire `RunEvent` into a slice-#10 command. */
    private dispatchWireEvent(ev: RunEventWire): void {
        switch (ev.kind) {
            case "run_started":
                // RunStarted already dispatched eagerly from `run()` —
                // the wire event is redundant. Skip to avoid double-firing.
                break;
            case "block_started":
                if (ev.block_id) {
                    this.dispatchIfAlive({
                        type: "BlockStarted",
                        blockId: ev.block_id,
                    });
                }
                break;
            case "block_done":
                if (ev.block_id) {
                    this.dispatchIfAlive({
                        type: "BlockDone",
                        blockId: ev.block_id,
                        output: ev.output,
                    });
                }
                break;
            case "block_error":
                if (ev.block_id) {
                    this.dispatchIfAlive({
                        type: "BlockError",
                        blockId: ev.block_id,
                        error: ev.error ?? "block failed",
                    });
                }
                break;
            case "run_done":
                this.dispatchIfAlive({
                    type: "RunDone",
                    output: ev.output ?? "",
                });
                break;
            case "run_failed":
                this.dispatchIfAlive({
                    type: "RunFailed",
                    error: ev.error ?? "run failed",
                });
                break;
        }
    }

    /** Look up the active run in the just-refreshed runs list and, if
     *  it's already terminal, dispatch `BackfilledFromRow` so the slot
     *  populates `blockResults` from persistence. Covers the codex P2
     *  race for fast drones whose events fire before we subscribe. */
    private maybeBackfillFromTerminalRun(runId: string, droneId: string): void {
        // If streaming events already populated, the reducer treats
        // backfill as authoritative — but the BlockDone events from the
        // stream are equally authoritative. Skip to avoid clobbering
        // a partial-but-correct mid-flight backlog.
        if (Object.keys(this.blockResultsAtom()).length > 0) return;
        const row = this.runsAtom().find((r) => r.id === runId);
        if (!row || (row.status !== "done" && row.status !== "failed")) return;
        const blocks = Object.entries(row.block_states ?? {}).map(
            ([blockId, st]) => ({
                blockId,
                status: st.status,
                output: st.output,
                error: st.error,
            }),
        );
        this.dispatchIfAlive(
            {
                type: "BackfilledFromRow",
                runId,
                droneId,
                status: row.status === "done" ? "done" : "failed",
                output: row.output,
                error: row.error,
                blocks,
            },
            "system",
        );
        if (this.activeRunUnsub) {
            this.activeRunUnsub();
            this.activeRunUnsub = null;
        }
    }

    async refreshRuns(droneId: string): Promise<void> {
        try {
            const list = await RpcApi.ListDroneRunsCommand(TabRpcClient, {
                drone_id: droneId,
                limit: 25,
            });
            this.setRuns(list);
        } catch {
            // non-fatal
        }
    }

    // ── Mutations on the draft graph ─────────────────────────────

    addNode(kind: BlockKind, position: { x: number; y: number }): FlowNode {
        const meta = blockMeta(kind);
        const node: FlowNode = {
            id: makeId(kind),
            position,
            data: { kind, ...meta.defaultData },
            type: kind,
        };
        // Append keeps `<For>`'s keying intact (new id → new row); the
        // existing node DOM is untouched.
        this.setDraftStore("graph", "nodes", (nodes) => [...nodes, node]);
        return node;
    }

    /** Add a node at the center of the currently visible canvas — the
     *  click-a-chip (no-drag) fallback. Inverts the viewport transform
     *  for the screen-center point, then offsets to roughly center the
     *  node body under that point. */
    addNodeAtCenter(kind: BlockKind): FlowNode {
        const v = this.viewportAtom();
        const { w, h } = this._canvasSize[0]();
        const cx = (w / 2 - v.x) / v.zoom - 80;
        const cy = (h / 2 - v.y) / v.zoom - 30;
        return this.addNode(kind, { x: cx, y: cy });
    }

    removeNode(id: string): void {
        // `produce` for the two-array cascade (drop the node + its edges)
        // in one granular transaction.
        this.setDraftStore(
            produce((d) => {
                d.graph.nodes = d.graph.nodes.filter((n) => n.id !== id);
                d.graph.edges = d.graph.edges.filter(
                    (e) => e.source !== id && e.target !== id,
                );
            }),
        );
        if (this.selectedAtom() === id) this.setSelected(null);
    }

    updateNodeData(id: string, patch: Record<string, unknown>): void {
        // Path mutation: touches only the matched node's `data` binding.
        this.setDraftStore(
            "graph",
            "nodes",
            (n) => n.id === id,
            "data",
            (data) => ({ ...data, ...patch }),
        );
    }

    moveNode(id: string, position: { x: number; y: number }): void {
        // The hot path during a drag — updates ONLY this node's
        // `position`, so its `<For>` row's transform binding re-runs and
        // nothing else does. This is the whole point of the store.
        this.setDraftStore("graph", "nodes", (n) => n.id === id, "position", position);
    }

    addEdge(edge: Omit<FlowEdge, "id">): void {
        const e: FlowEdge = { id: `e_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`, ...edge };
        this.setDraftStore("graph", "edges", (edges) => [...edges, e]);
    }

    removeEdge(id: string): void {
        this.setDraftStore("graph", "edges", (edges) => edges.filter((e) => e.id !== id));
        if (this.selectedEdgeAtom() === id) this.setSelectedEdge(null);
    }

    /** Validate a proposed connection before it becomes an edge:
     *  - no self-loop,
     *  - no duplicate (same source/handle → same target/handle),
     *  - compatible port types (`any` matches anything),
     *  - no cycle (keeps the graph a DAG for the executor's topo sort). */
    canConnect(c: {
        source: string;
        sourceHandle?: string;
        target: string;
        targetHandle?: string;
    }): boolean {
        if (c.source === c.target) return false;
        const graph = this.draftAtom().graph;
        const src = graph.nodes.find((n) => n.id === c.source);
        const dst = graph.nodes.find((n) => n.id === c.target);
        if (!src || !dst) return false;
        const srcH = c.sourceHandle ?? "out";
        const dstH = c.targetHandle ?? "in";
        // Single-input occupancy: each declared input handle takes exactly
        // one wire, so reject a second edge into an already-wired input
        // (this also covers exact duplicates). A multi-incoming join would
        // require the registry to declare multiple input handles.
        if (graph.edges.some((e) => e.target === c.target && (e.targetHandle ?? "in") === dstH))
            return false;
        const outType =
            blockMeta(src.data.kind as BlockKind).outputs.find((h) => h.id === srcH)?.type ?? "any";
        const inType =
            blockMeta(dst.data.kind as BlockKind).inputs.find((h) => h.id === dstH)?.type ?? "any";
        if (outType !== "any" && inType !== "any" && outType !== inType) return false;
        // Cycle check: if the target can already reach the source, adding
        // source→target would close a loop.
        if (this.reaches(c.target, c.source)) return false;
        return true;
    }

    /** Can `from` reach `to` by following edge direction? (iterative DFS) */
    private reaches(from: string, to: string): boolean {
        const edges = this.draftAtom().graph.edges;
        const seen = new Set<string>();
        const stack = [from];
        while (stack.length) {
            const cur = stack.pop() as string;
            if (cur === to) return true;
            if (seen.has(cur)) continue;
            seen.add(cur);
            for (const e of edges) if (e.source === cur) stack.push(e.target);
        }
        return false;
    }

    /** Validate then add an edge. Returns whether it was added. */
    connect(c: {
        source: string;
        sourceHandle?: string;
        target: string;
        targetHandle?: string;
    }): boolean {
        if (!this.canConnect(c)) return false;
        this.addEdge(c);
        return true;
    }

    setName(name: string): void {
        this.setDraftStore("name", name);
    }

    /** Validation surface read by the NodeTypeBar before enabling Run. */
    validate(): { ok: boolean; errors: string[] } {
        const errors: string[] = [];
        const graph = this.draftAtom().graph;
        if (graph.nodes.length === 0) {
            errors.push("Drone is empty.");
        }
        const responses = graph.nodes.filter((n) => n.data.kind === "response").length;
        if (responses === 0) errors.push("Drone needs exactly one Response block.");
        if (responses > 1) errors.push("Only one Response block per drone.");
        return { ok: errors.length === 0, errors };
    }

    dispose(): void {
        if (this.disposed) return;
        this.disposed = true;
        if (this.activeRunUnsub) {
            this.activeRunUnsub();
            this.activeRunUnsub = null;
        }
        dispatchDroneRun(this.blockId, { type: "Disposed" });
        unregisterPane(this.blockId);
    }

    /** Guarded dispatch — every async path that could resolve after
     *  `dispose()` (e.g. an in-flight `run()` whose `refreshRuns`
     *  finishes after the pane unmounts) routes through this helper
     *  so the dispatch never lands on an unregistered slot.
     *  Codex P2 on PR #844 (dispose idempotency) generalized: the
     *  same race exists for ANY post-dispose dispatch, not just a
     *  second `Disposed`. */
    private dispatchIfAlive(
        command: Parameters<typeof dispatchDroneRun>[1],
        source?: Parameters<typeof dispatchDroneRun>[2],
    ): void {
        if (this.disposed) return;
        dispatchDroneRun(this.blockId, command, source);
    }
}

/** Shape of one `dronerun:<id>` event payload as emitted by
 *  `agentmux-srv/src/drone/executor/engine.rs::RunEvent`
 *  (`#[serde(tag = "kind", rename_all = "snake_case")]`). */
interface RunEventWire {
    kind:
        | "run_started"
        | "block_started"
        | "block_done"
        | "block_error"
        | "run_done"
        | "run_failed";
    run_id?: string;
    drone_id?: string;
    block_id?: string;
    output?: unknown;
    error?: string;
}

function makeId(prefix: string): string {
    return `${prefix}_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`;
}

// Re-export the slot's per-block result type for view consumers that
// still import from the view-model module.
export type { AgentBlockResult } from "@/app/store/drone-run-state-store";
