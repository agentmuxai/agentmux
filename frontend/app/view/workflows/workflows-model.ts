// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// WorkflowsViewModel — owns the per-pane state for the Workflows widget.
//
// Phase 1.5 PR 4 (`docs/specs/SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md`
// §6 row 4) routes per-run state through the `workflow-run-state` slice
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
    dispatch as dispatchWorkflowRun,
    registerPane,
    unregisterPane,
    type AgentBlockResult,
    type WorkflowRunStatus,
} from "@/app/store/workflow-run-state-store";
import { waveEventSubscribe } from "@/app/store/wps";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, createSignal, type Accessor } from "solid-js";

import { blockMeta } from "./block-registry";
import {
    defaultViewport,
    emptyGraph,
    type BlockKind,
    type FlowEdge,
    type FlowNode,
    type WorkflowDefinition,
    type WorkflowGraph,
    type WorkflowRun,
} from "./workflows-types";

const BLANK_WORKFLOW = (): WorkflowDefinition => ({
    id: "",
    name: "Untitled Workflow",
    description: "",
    graph: emptyGraph(),
    viewport: defaultViewport(),
    created_at: 0,
    updated_at: 0,
});

export class WorkflowsViewModel implements ViewModel {
    viewType = "workflows";
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

    // --- list of saved workflows
    private _list = createSignal<WorkflowDefinition[]>([]);
    listAtom: Accessor<WorkflowDefinition[]> = this._list[0];
    setList = this._list[1];

    // --- the workflow currently open in the canvas (a draft until saved)
    private _draft = createSignal<WorkflowDefinition>(BLANK_WORKFLOW());
    draftAtom: Accessor<WorkflowDefinition> = this._draft[0];
    setDraft = this._draft[1];

    // --- which node is selected in the canvas (for the InspectorPanel)
    private _selected = createSignal<string | null>(null);
    selectedAtom: Accessor<string | null> = this._selected[0];
    setSelected = this._selected[1];

    // --- run state
    //
    // `_running` is the in-flight `await RunWorkflowCommand` flag — bound
    // to the Toolbar's button disable. It's a UI thing, separate from the
    // slot's `status` (which is "idle"|"running"|"done"|"failed", folded
    // from `workflowrun:<id>` events). Keeping it in the view.
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

    private _status = createSignal<WorkflowRunStatus>("idle");
    statusAtom: Accessor<WorkflowRunStatus> = this._status[0];

    private _blockResults = createSignal<Record<string, AgentBlockResult>>({});
    blockResultsAtom: Accessor<Record<string, AgentBlockResult>> = this._blockResults[0];

    blockResultAtom(blockId: string): AgentBlockResult | undefined {
        return this.blockResultsAtom()[blockId];
    }

    // ── Non-reducer-backed view-only cells ─────────────────────────
    private _runs = createSignal<WorkflowRun[]>([]);
    runsAtom: Accessor<WorkflowRun[]> = this._runs[0];
    setRuns = this._runs[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    // Active `workflowrun:<id>` subscription. Stored so we can
    // unsubscribe on dispose / when the run changes.
    private activeRunUnsub: (() => void) | null = null;

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
            workflowId: () => {
                // Not mirrored — the view reads draft.id, not the slot's
                // workflowId. Keeping the projection a no-op so the slot
                // surface stays uniform with slice #9.
            },
            status: (next) => this._status[1](next),
            blockResults: (next) => this._blockResults[1](next),
            output: () => {
                // The terminal `output` projection is unused today — the
                // Run panel reads the runs-list row instead, and the
                // inspector reads blockResults. Reserved for Phase 2
                // when a workflow-level result panel lands.
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
            const list = await RpcApi.ListWorkflowsCommand(TabRpcClient, {});
            this.setList(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load workflows: ${(e as Error).message ?? e}`);
        }
    }

    /** Load a workflow into the canvas as the draft. */
    async openWorkflow(id: string): Promise<void> {
        try {
            const wf = await RpcApi.GetWorkflowCommand(TabRpcClient, { id });
            if (wf) {
                this.setDraft(wf);
                this.setSelected(null);
                dispatchWorkflowRun(this.blockId, { type: "Reset" }, "user");
                await this.refreshRuns(id);
            }
        } catch (e) {
            this.setError(`Failed to open workflow: ${(e as Error).message ?? e}`);
        }
    }

    /** Start a fresh blank workflow in the canvas. */
    newWorkflow(): void {
        this.setDraft(BLANK_WORKFLOW());
        this.setSelected(null);
        this.setRuns([]);
        dispatchWorkflowRun(this.blockId, { type: "Reset" }, "user");
        if (this.activeRunUnsub) {
            this.activeRunUnsub();
            this.activeRunUnsub = null;
        }
    }

    /** Persist the current draft. Returns the saved id. */
    async save(): Promise<string | null> {
        const wf = this.draftAtom();
        try {
            const saved = await RpcApi.UpsertWorkflowCommand(TabRpcClient, wf);
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
        try {
            const r = await RpcApi.RunWorkflowCommand(TabRpcClient, { workflow_id: id });
            dispatchWorkflowRun(
                this.blockId,
                { type: "RunStarted", runId: r.run_id, workflowId: id },
                "user",
            );
            this.subscribeRun(r.run_id, id);
            // Backend inserts a `running` placeholder row synchronously
            // before this RPC resolves; the subscription above picks up
            // `RunDone` / `RunFailed` and re-refreshes the runs list
            // (Phase 1.5 PR 3, #830).
            await this.refreshRuns(id);
            // Race recovery: ultra-fast workflows can finish + persist
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

    /** Subscribe to `workflowrun:<runId>` WPS events for the active run.
     *  Replaces any prior subscription. The backend publishes one event
     *  per `RunEvent` variant — we route every event into the slot
     *  reducer and refresh the run row on terminal events. */
    private subscribeRun(runId: string, workflowId: string): void {
        if (this.activeRunUnsub) {
            this.activeRunUnsub();
            this.activeRunUnsub = null;
        }
        this.activeRunUnsub = waveEventSubscribe({
            eventType: `workflowrun:${runId}`,
            scope: "",
            handler: (event) => {
                const data = (event as { data?: RunEventWire }).data;
                if (!data) return;
                this.dispatchWireEvent(data);
                if (data.kind === "run_done" || data.kind === "run_failed") {
                    // Backend writes the row update BEFORE publishing
                    // the terminal event (codex P2 on #843), so a
                    // refresh here lands the final state.
                    void this.refreshRuns(workflowId);
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
                    dispatchWorkflowRun(this.blockId, {
                        type: "BlockStarted",
                        blockId: ev.block_id,
                    });
                }
                break;
            case "block_done":
                if (ev.block_id) {
                    dispatchWorkflowRun(this.blockId, {
                        type: "BlockDone",
                        blockId: ev.block_id,
                        output: ev.output,
                    });
                }
                break;
            case "block_error":
                if (ev.block_id) {
                    dispatchWorkflowRun(this.blockId, {
                        type: "BlockError",
                        blockId: ev.block_id,
                        error: ev.error ?? "block failed",
                    });
                }
                break;
            case "run_done":
                dispatchWorkflowRun(this.blockId, {
                    type: "RunDone",
                    output: ev.output ?? "",
                });
                break;
            case "run_failed":
                dispatchWorkflowRun(this.blockId, {
                    type: "RunFailed",
                    error: ev.error ?? "run failed",
                });
                break;
        }
    }

    /** Look up the active run in the just-refreshed runs list and, if
     *  it's already terminal, dispatch `BackfilledFromRow` so the slot
     *  populates `blockResults` from persistence. Covers the codex P2
     *  race for fast workflows whose events fire before we subscribe. */
    private maybeBackfillFromTerminalRun(runId: string, workflowId: string): void {
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
        dispatchWorkflowRun(
            this.blockId,
            {
                type: "BackfilledFromRow",
                runId,
                workflowId,
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

    async refreshRuns(workflowId: string): Promise<void> {
        try {
            const list = await RpcApi.ListWorkflowRunsCommand(TabRpcClient, {
                workflow_id: workflowId,
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
        this.setDraft((prev) => ({
            ...prev,
            graph: { ...prev.graph, nodes: [...prev.graph.nodes, node] },
        }));
        return node;
    }

    removeNode(id: string): void {
        this.setDraft((prev) => ({
            ...prev,
            graph: {
                nodes: prev.graph.nodes.filter((n) => n.id !== id),
                edges: prev.graph.edges.filter((e) => e.source !== id && e.target !== id),
            },
        }));
        if (this.selectedAtom() === id) this.setSelected(null);
    }

    updateNodeData(id: string, patch: Record<string, unknown>): void {
        this.setDraft((prev) => ({
            ...prev,
            graph: {
                ...prev.graph,
                nodes: prev.graph.nodes.map((n) =>
                    n.id === id ? { ...n, data: { ...n.data, ...patch } } : n,
                ),
            },
        }));
    }

    moveNode(id: string, position: { x: number; y: number }): void {
        this.setDraft((prev) => ({
            ...prev,
            graph: {
                ...prev.graph,
                nodes: prev.graph.nodes.map((n) =>
                    n.id === id ? { ...n, position } : n,
                ),
            },
        }));
    }

    addEdge(edge: Omit<FlowEdge, "id">): void {
        const e: FlowEdge = { id: `e_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`, ...edge };
        this.setDraft((prev) => ({
            ...prev,
            graph: { ...prev.graph, edges: [...prev.graph.edges, e] },
        }));
    }

    removeEdge(id: string): void {
        this.setDraft((prev) => ({
            ...prev,
            graph: { ...prev.graph, edges: prev.graph.edges.filter((e) => e.id !== id) },
        }));
    }

    setName(name: string): void {
        this.setDraft((prev) => ({ ...prev, name }));
    }

    /** Validation surface read by the Toolbar before enabling Run. */
    validate(): { ok: boolean; errors: string[] } {
        const errors: string[] = [];
        const graph = this.draftAtom().graph;
        if (graph.nodes.length === 0) {
            errors.push("Workflow is empty.");
        }
        const responses = graph.nodes.filter((n) => n.data.kind === "response").length;
        if (responses === 0) errors.push("Workflow needs exactly one Response block.");
        if (responses > 1) errors.push("Only one Response block per workflow.");
        return { ok: errors.length === 0, errors };
    }

    dispose(): void {
        if (this.activeRunUnsub) {
            this.activeRunUnsub();
            this.activeRunUnsub = null;
        }
        dispatchWorkflowRun(this.blockId, { type: "Disposed" });
        unregisterPane(this.blockId);
    }
}

/** Shape of one `workflowrun:<id>` event payload as emitted by
 *  `agentmux-srv/src/workflows/executor/engine.rs::RunEvent`
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
    workflow_id?: string;
    block_id?: string;
    output?: unknown;
    error?: string;
}

function makeId(prefix: string): string {
    return `${prefix}_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`;
}

// Re-export the slot's per-block result type for view consumers that
// still import from the view-model module.
export type { AgentBlockResult } from "@/app/store/workflow-run-state-store";
