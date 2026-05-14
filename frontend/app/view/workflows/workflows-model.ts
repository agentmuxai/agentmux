// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// WorkflowsViewModel — owns the per-pane state for the Workflows widget.
// Mirrors MemoryViewModel's pattern (see frontend/app/view/memory/memory-model.ts):
// SolidJS signals for UI state, RPC calls for persistence + run, no
// global stores.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
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
    private _running = createSignal<boolean>(false);
    runningAtom: Accessor<boolean> = this._running[0];
    setRunning = this._running[1];

    private _runEvents = createSignal<RunEventEntry[]>([]);
    runEventsAtom: Accessor<RunEventEntry[]> = this._runEvents[0];
    setRunEvents = this._runEvents[1];

    private _activeRunId = createSignal<string | null>(null);
    activeRunIdAtom: Accessor<string | null> = this._activeRunId[0];
    setActiveRunId = this._activeRunId[1];

    private _runs = createSignal<WorkflowRun[]>([]);
    runsAtom: Accessor<WorkflowRun[]> = this._runs[0];
    setRuns = this._runs[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    // ── Per-block last-run result (Phase 1.5 PR 3 — #830) ──────────
    //
    // Keyed by the workflow block id. Populated from `BlockDone` /
    // `BlockError` events on the active run; cleared on `newWorkflow`
    // and when the user starts a new run.
    private _blockResults = createSignal<Record<string, AgentBlockResult>>({});
    blockResultsAtom: Accessor<Record<string, AgentBlockResult>> = this._blockResults[0];
    setBlockResults = this._blockResults[1];

    blockResultAtom(blockId: string): AgentBlockResult | undefined {
        return this.blockResultsAtom()[blockId];
    }

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
                this.setRunEvents([]);
                this.setActiveRunId(null);
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
        this.setRunEvents([]);
        this.setActiveRunId(null);
        this.setRuns([]);
        this.setBlockResults({});
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
        this.setRunEvents([]);
        this.setBlockResults({});
        try {
            const r = await RpcApi.RunWorkflowCommand(TabRpcClient, { workflow_id: id });
            this.setActiveRunId(r.run_id);
            this.subscribeRun(r.run_id, id);
            // Backend inserts a `running` placeholder row synchronously
            // before this RPC resolves; the subscription above picks up
            // the `RunDone` / `RunFailed` transition and re-refreshes
            // the runs list (Phase 1.5 PR 3, closes #830).
            await this.refreshRuns(id);
        } catch (e) {
            this.setError(`Run failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setRunning(false);
        }
    }

    /** Subscribe to `workflowrun:<runId>` WPS events for the active run.
     *  Replaces any prior subscription. The backend publishes one event
     *  per `RunEvent` variant — we drive the run-panel + per-block
     *  result cache off them and refresh the run row on terminal events. */
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
                this.applyRunEvent(data);
                if (data.kind === "run_done" || data.kind === "run_failed") {
                    // Refresh once the placeholder row has been UPDATEd
                    // by the backend's drain task. Backend writes the
                    // row after sending the terminal event, so a small
                    // microtask delay is enough.
                    void this.refreshRuns(workflowId);
                    if (this.activeRunUnsub) {
                        this.activeRunUnsub();
                        this.activeRunUnsub = null;
                    }
                }
            },
        });
    }

    /** Fold one `RunEvent` into the model's reactive state. Kept in the
     *  view model for Phase 1.5; slice #10 (PR 4) will move this into a
     *  pure reducer per the master reducer-stack pattern. */
    private applyRunEvent(ev: RunEventWire): void {
        switch (ev.kind) {
            case "block_done":
                if (ev.block_id) {
                    this.setBlockResults((prev) => ({
                        ...prev,
                        [ev.block_id!]: parseBlockOutput(ev.output),
                    }));
                }
                break;
            case "block_error":
                if (ev.block_id) {
                    this.setBlockResults((prev) => ({
                        ...prev,
                        [ev.block_id!]: { response: "", error: ev.error ?? "block failed" },
                    }));
                }
                break;
            default:
                break;
        }
        this.appendRunEvent({
            kind: ev.kind,
            block_id: ev.block_id,
            output: ev.output,
            error: ev.error,
            at: Date.now(),
        });
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

    appendRunEvent(ev: RunEventEntry): void {
        this.setRunEvents((prev) => [...prev, ev]);
    }

    clearRunEvents(): void {
        this.setRunEvents([]);
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
    }
}

export interface RunEventEntry {
    kind: string;
    block_id?: string;
    output?: unknown;
    error?: string;
    at: number;
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

/** What we render in the Agent block inspector for the last run. */
export interface AgentBlockResult {
    response: string;
    costUsd?: number;
    error?: string;
}

/** Pull the response text + cost out of the `BlockDone.output` shape
 *  emitted by `workflows/executor/blocks/agent.rs` (spec §4.3). The
 *  block returns `{ response, tokens, cost_usd }` — variables/api/etc.
 *  blocks return arbitrary shapes that we fall back to JSON-stringify. */
function parseBlockOutput(output: unknown): AgentBlockResult {
    if (output && typeof output === "object") {
        const o = output as Record<string, unknown>;
        if (typeof o["response"] === "string") {
            return {
                response: o["response"],
                costUsd: typeof o["cost_usd"] === "number" ? (o["cost_usd"] as number) : undefined,
            };
        }
    }
    return { response: typeof output === "string" ? output : JSON.stringify(output ?? null) };
}

function makeId(prefix: string): string {
    return `${prefix}_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`;
}
