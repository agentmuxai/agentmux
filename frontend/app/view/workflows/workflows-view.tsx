// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createResource, For, Show, type JSX } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { BLOCK_KINDS, blockMeta } from "./block-registry";
import type { WorkflowsViewModel } from "./workflows-model";
import type { BlockKind, FlowNode } from "./workflows-types";
import "./workflows-view.scss";

export const WorkflowsView = (props: { model: WorkflowsViewModel }): JSX.Element => {
    const m = props.model;

    return (
        <div class="workflows-pane">
            <Toolbar model={m} />
            <div class="workflows-body">
                <BlockPalette model={m} />
                <div class="workflows-canvas-wrap">
                    <Canvas model={m} />
                </div>
                <InspectorPanel model={m} />
            </div>
            <RunPanel model={m} />
        </div>
    );
};

WorkflowsView.displayName = "WorkflowsView";

// ── Toolbar ───────────────────────────────────────────────────────────

const Toolbar = (p: { model: WorkflowsViewModel }): JSX.Element => {
    const m = p.model;
    const validation = () => m.validate();
    return (
        <header class="workflows-toolbar">
            <input
                class="workflows-toolbar-name"
                value={m.draftAtom().name}
                onInput={(e) => m.setName(e.currentTarget.value)}
                placeholder="Untitled Workflow"
                aria-label="Workflow name"
            />
            <div class="workflows-toolbar-actions">
                <button class="workflows-btn" onClick={() => m.newWorkflow()}>
                    New
                </button>
                <button class="workflows-btn" onClick={() => void m.save()}>
                    Save
                </button>
                <button
                    class="workflows-btn workflows-btn--primary"
                    disabled={!validation().ok || m.runningAtom()}
                    title={validation().errors.join(" · ")}
                    onClick={() => void m.run()}
                >
                    {m.runningAtom() ? "Running…" : "Run"}
                </button>
            </div>
            <Show when={m.errorAtom()}>
                <div class="workflows-error" role="alert">
                    {m.errorAtom()}
                </div>
            </Show>
        </header>
    );
};

// ── Block palette ─────────────────────────────────────────────────────

const BlockPalette = (p: { model: WorkflowsViewModel }): JSX.Element => {
    const m = p.model;
    return (
        <aside class="workflows-palette" aria-label="Block palette">
            <div class="workflows-palette-title">Blocks</div>
            <For each={BLOCK_KINDS}>
                {(kind) => {
                    const meta = blockMeta(kind);
                    return (
                        <button
                            class="workflows-palette-item"
                            style={{ "--block-color": meta.color }}
                            onClick={() => {
                                // Naive placement — Phase 2 will use drag + drop
                                // with cursor coords. For now, drop near origin
                                // with a small random offset so adds don't stack.
                                m.addNode(kind, {
                                    x: 80 + Math.random() * 60,
                                    y: 80 + Math.random() * 60,
                                });
                            }}
                            title={meta.description}
                        >
                            <span class="workflows-palette-item-dot" />
                            <span class="workflows-palette-item-label">{meta.label}</span>
                        </button>
                    );
                }}
            </For>
            <Show when={m.listAtom().length > 0}>
                <div class="workflows-palette-section">Saved workflows</div>
                <For each={m.listAtom()}>
                    {(wf) => (
                        <button
                            class="workflows-palette-item workflows-palette-item--workflow"
                            onClick={() => void m.openWorkflow(wf.id)}
                        >
                            <span class="workflows-palette-item-label">{wf.name}</span>
                        </button>
                    )}
                </For>
            </Show>
        </aside>
    );
};

// ── Canvas ────────────────────────────────────────────────────────────
//
// Phase 1 ships a minimal SVG-based canvas instead of the full
// `@dschz/solid-flow` integration. Rationale: the xyflow runtime expects
// SolidJS reactive primitives wired through a `<SolidFlow>` provider
// component which conflicts with our existing Solid root setup; the
// integration spike is its own follow-up. This canvas covers the demo
// path (drop nodes, click to select, draw edges by clicking source then
// target). The Phase 1 PR-4 polish issue tracks the swap.

const Canvas = (p: { model: WorkflowsViewModel }): JSX.Element => {
    const m = p.model;
    let edgeStartId: string | null = null;

    const onNodeClick = (e: MouseEvent, n: FlowNode) => {
        e.stopPropagation();
        if (e.shiftKey) {
            // Shift-click toggles edge-draw mode.
            if (edgeStartId == null) {
                edgeStartId = n.id;
                return;
            }
            if (edgeStartId !== n.id) {
                // For edges sourced from a Condition block, auto-assign
                // the branch handle by edge-order: first connection =
                // "true" branch, second = "false". The executor's
                // branch-pruning logic (engine.rs `edge_is_active`)
                // gates on this handle, so without it BOTH branches
                // run and fire API/agent side effects on the path
                // that should have been skipped. Phase 1 trades
                // discoverability for simplicity; Phase 2 / canvas
                // polish PR introduces a UI affordance (color, label,
                // right-click "swap branches"). See codex review on
                // PR #755.
                const srcId = edgeStartId;
                const srcNode = m
                    .draftAtom()
                    .graph.nodes.find((x) => x.id === srcId);
                const isCondition =
                    (srcNode?.data?.kind as string | undefined) === "condition";
                let sourceHandle: string | undefined;
                if (isCondition) {
                    const existing = m
                        .draftAtom()
                        .graph.edges.filter((edge) => edge.source === srcId);
                    sourceHandle = existing.length === 0 ? "true" : "false";
                }
                m.addEdge({
                    source: srcId,
                    target: n.id,
                    ...(sourceHandle ? { sourceHandle } : {}),
                });
            }
            edgeStartId = null;
            return;
        }
        m.setSelected(n.id);
    };

    return (
        <div class="workflows-canvas" onClick={() => m.setSelected(null)}>
            <svg class="workflows-canvas-edges" aria-hidden="true">
                <For each={m.draftAtom().graph.edges}>
                    {(edge) => {
                        const src = () =>
                            m.draftAtom().graph.nodes.find((n) => n.id === edge.source);
                        const dst = () =>
                            m.draftAtom().graph.nodes.find((n) => n.id === edge.target);
                        return (
                            <Show when={src() && dst()}>
                                <line
                                    class="workflows-edge"
                                    x1={src()!.position.x + 80}
                                    y1={src()!.position.y + 28}
                                    x2={dst()!.position.x}
                                    y2={dst()!.position.y + 28}
                                />
                            </Show>
                        );
                    }}
                </For>
            </svg>
            <For each={m.draftAtom().graph.nodes}>
                {(n) => {
                    const meta = blockMeta(n.data.kind as BlockKind);
                    const selected = () => m.selectedAtom() === n.id;
                    return (
                        <div
                            class="workflows-node"
                            classList={{
                                "workflows-node--selected": selected(),
                            }}
                            style={{
                                left: `${n.position.x}px`,
                                top: `${n.position.y}px`,
                                "--block-color": meta.color,
                            }}
                            onClick={(e) => onNodeClick(e, n)}
                        >
                            <header class="workflows-node-header">
                                <span class="workflows-node-label">{meta.label}</span>
                                <button
                                    class="workflows-node-close"
                                    onClick={(e) => {
                                        e.stopPropagation();
                                        m.removeNode(n.id);
                                    }}
                                    aria-label="Remove block"
                                >
                                    ×
                                </button>
                            </header>
                            <div class="workflows-node-body">{nodeSummary(n)}</div>
                        </div>
                    );
                }}
            </For>
            <div class="workflows-canvas-hint">
                Click to add blocks · Shift-click two nodes to connect them
            </div>
        </div>
    );
};

function nodeSummary(n: FlowNode): string {
    switch (n.data.kind) {
        case "agent": {
            const ref = readAgentRef(n);
            const task = (n.data["task"] as string) ?? "";
            const who = ref.instanceName || ref.identityId || "blank";
            return `${who} · ${truncate(task)}`;
        }
        case "api":
            return `${(n.data["method"] as string) ?? "GET"} ${truncate((n.data["url"] as string) ?? "")}`;
        case "condition":
            return `if ${truncate((n.data["expr"] as string) ?? "")}`;
        case "response":
            return truncate((n.data["template"] as string) ?? "<empty>");
        case "variables": {
            const entries = (n.data["entries"] as Array<{ name: string }> | undefined) ?? [];
            return entries.length > 0
                ? entries.map((e) => e.name).join(", ")
                : "<no entries>";
        }
        default:
            return "";
    }
}

function truncate(s: string, max = 40): string {
    return s.length > max ? s.slice(0, max - 1) + "…" : s;
}

// ── Inspector ─────────────────────────────────────────────────────────

const InspectorPanel = (p: { model: WorkflowsViewModel }): JSX.Element => {
    const m = p.model;
    return (
        <aside class="workflows-inspector" aria-label="Inspector">
            <Show
                when={m.selectedNodeAtom()}
                fallback={
                    <div class="workflows-inspector-empty">Select a block to edit.</div>
                }
            >
                {(node) => <InspectorForm model={m} node={node()} />}
            </Show>
        </aside>
    );
};

const InspectorForm = (p: { model: WorkflowsViewModel; node: FlowNode }): JSX.Element => {
    const meta = blockMeta(p.node.data.kind as BlockKind);
    const update = (patch: Record<string, unknown>) =>
        p.model.updateNodeData(p.node.id, patch);

    return (
        <div class="workflows-inspector-form">
            <div class="workflows-inspector-title">{meta.label}</div>
            <div class="workflows-inspector-id">{p.node.id}</div>
            <Show when={p.node.data.kind === "agent"}>
                <AgentRefEditor node={p.node} update={update} />
                <Field label="Task ({{...}} interpolation supported)">
                    <textarea
                        class="workflows-input"
                        rows="4"
                        value={(p.node.data["task"] as string) ?? ""}
                        onInput={(e) => update({ task: e.currentTarget.value })}
                    />
                </Field>
                <AgentResultPanel model={p.model} blockId={p.node.id} />
            </Show>
            <Show when={p.node.data.kind === "api"}>
                <Field label="Method">
                    <select
                        class="workflows-input"
                        value={(p.node.data["method"] as string) ?? "GET"}
                        onChange={(e) => update({ method: e.currentTarget.value })}
                    >
                        <option>GET</option>
                        <option>POST</option>
                        <option>PUT</option>
                        <option>PATCH</option>
                        <option>DELETE</option>
                    </select>
                </Field>
                <Field label="URL">
                    <input
                        class="workflows-input"
                        value={(p.node.data["url"] as string) ?? ""}
                        onInput={(e) => update({ url: e.currentTarget.value })}
                        placeholder="https://example.com/{{var.endpoint}}"
                    />
                </Field>
                <Field label="Body">
                    <textarea
                        class="workflows-input"
                        rows="4"
                        value={(p.node.data["body"] as string) ?? ""}
                        onInput={(e) => update({ body: e.currentTarget.value })}
                    />
                </Field>
            </Show>
            <Show when={p.node.data.kind === "condition"}>
                <Field label="Expression (e.g. {{var.x}} > 10)">
                    <input
                        class="workflows-input"
                        value={(p.node.data["expr"] as string) ?? ""}
                        onInput={(e) => update({ expr: e.currentTarget.value })}
                        placeholder="{{var.count}} > 0"
                    />
                </Field>
            </Show>
            <Show when={p.node.data.kind === "response"}>
                <Field label="Template (final workflow output)">
                    <textarea
                        class="workflows-input"
                        rows="4"
                        value={(p.node.data["template"] as string) ?? ""}
                        onInput={(e) => update({ template: e.currentTarget.value })}
                        placeholder="Hello {{var.name}}!"
                    />
                </Field>
            </Show>
            <Show when={p.node.data.kind === "variables"}>
                <VariablesEditor
                    entries={
                        (p.node.data["entries"] as Array<{ name: string; value: string }>) ?? []
                    }
                    onChange={(entries) => update({ entries })}
                />
            </Show>
        </div>
    );
};

const Field = (p: { label: string; children: JSX.Element }): JSX.Element => (
    <label class="workflows-field">
        <span class="workflows-field-label">{p.label}</span>
        {p.children}
    </label>
);

const VariablesEditor = (p: {
    entries: Array<{ name: string; value: string }>;
    onChange: (next: Array<{ name: string; value: string }>) => void;
}): JSX.Element => {
    const update = (i: number, patch: Partial<{ name: string; value: string }>) => {
        p.onChange(p.entries.map((e, idx) => (idx === i ? { ...e, ...patch } : e)));
    };
    return (
        <div class="workflows-vars">
            <For each={p.entries}>
                {(entry, i) => (
                    <div class="workflows-vars-row">
                        <input
                            class="workflows-input"
                            value={entry.name}
                            onInput={(e) => update(i(), { name: e.currentTarget.value })}
                            placeholder="name"
                        />
                        <input
                            class="workflows-input"
                            value={entry.value}
                            onInput={(e) => update(i(), { value: e.currentTarget.value })}
                            placeholder="value"
                        />
                        <button
                            class="workflows-btn workflows-btn--small"
                            onClick={() => p.onChange(p.entries.filter((_, idx) => idx !== i()))}
                        >
                            ×
                        </button>
                    </div>
                )}
            </For>
            <button
                class="workflows-btn workflows-btn--small"
                onClick={() => p.onChange([...p.entries, { name: "", value: "" }])}
            >
                + Add
            </button>
        </div>
    );
};

// ── AgentRef editor ───────────────────────────────────────────────────
//
// PR 3 of Phase 1.5 (closes #835). Replaces the prior single
// `forge_agent_id` text field with separate identity / memory /
// instance-name pickers backed by the launch-modal RPCs.

interface AgentRefShape {
    identityId: string;
    memoryId: string;
    instanceName: string;
    workingDirectory: string;
}

function readAgentRef(n: FlowNode): AgentRefShape {
    const raw = n.data["agent_ref"] as Partial<AgentRefShape> | undefined;
    if (raw && typeof raw === "object") {
        return {
            identityId: raw.identityId ?? "",
            memoryId: raw.memoryId ?? "",
            instanceName: raw.instanceName ?? "",
            workingDirectory: raw.workingDirectory ?? "",
        };
    }
    // Legacy pre-#835 nodes persisted `forge_agent_id`. Phase 1.5 PR 2
    // (#834) wired the executor to read `agent_ref` only — a legacy
    // node now launches blank claude. Surface that explicitly rather
    // than silently coercing the old id into anything.
    const legacy = n.data["forge_agent_id"];
    if (typeof legacy === "string" && legacy.length > 0) {
        console.warn(
            `[workflows] Agent block ${n.id} uses legacy forge_agent_id="${legacy}"; re-pick identity/memory after PR 3.`,
        );
    }
    return { identityId: "", memoryId: "", instanceName: "", workingDirectory: "" };
}

const AgentRefEditor = (p: {
    node: FlowNode;
    update: (patch: Record<string, unknown>) => void;
}): JSX.Element => {
    const [identities] = createResource(() =>
        RpcApi.ListIdentityBundlesCommand(TabRpcClient, {}).catch(() => [] as IdentityBundle[]),
    );
    const [memories] = createResource(() =>
        RpcApi.ListMemoriesCommand(TabRpcClient, {}).catch(() => [] as Memory[]),
    );
    const ref = () => readAgentRef(p.node);
    const setRef = (patch: Partial<AgentRefShape>) =>
        p.update({ agent_ref: { ...ref(), ...patch } });

    return (
        <>
            <Field label="Identity">
                <select
                    class="workflows-input"
                    value={ref().identityId}
                    onChange={(e) => setRef({ identityId: e.currentTarget.value })}
                >
                    <option value="">— Blank (ambient creds) —</option>
                    <For each={(identities() ?? []).filter((b) => !b.is_blank)}>
                        {(bundle) => <option value={bundle.id}>{bundle.name}</option>}
                    </For>
                </select>
            </Field>
            <Field label="Memory">
                <select
                    class="workflows-input"
                    value={ref().memoryId}
                    onChange={(e) => setRef({ memoryId: e.currentTarget.value })}
                >
                    <option value="">— Blank (vanilla CLI) —</option>
                    <For each={(memories() ?? []).filter((m) => !m.is_blank)}>
                        {(memory) => <option value={memory.id}>{memory.name}</option>}
                    </For>
                </select>
            </Field>
            <Field label="Instance name (optional, for named-agent continuation)">
                <input
                    class="workflows-input"
                    value={ref().instanceName}
                    onInput={(e) => setRef({ instanceName: e.currentTarget.value })}
                    placeholder="leave blank for one-shot"
                />
            </Field>
        </>
    );
};

// ── Agent result panel ────────────────────────────────────────────────
//
// Shows the most recent run's BlockDone output for the selected Agent
// block. Subscribed via the model (`workflowrun:<id>` events → §5.2).
// Phase 1.5 ships final-result rendering only; hover-expand tool stream
// is deferred to Phase 2 polish.

const AgentResultPanel = (p: {
    model: WorkflowsViewModel;
    blockId: string;
}): JSX.Element => {
    const result = () => p.model.blockResultAtom(p.blockId);
    return (
        <Show when={result()}>
            {(r) => (
                <div class="workflows-agent-result">
                    <div class="workflows-agent-result-label">Last run</div>
                    <Show
                        when={r().error}
                        fallback={
                            <>
                                <pre class="workflows-agent-result-text">{r().response}</pre>
                                <Show when={r().costUsd != null}>
                                    <div class="workflows-agent-result-cost">
                                        ${r().costUsd!.toFixed(4)}
                                    </div>
                                </Show>
                            </>
                        }
                    >
                        <pre class="workflows-agent-result-error">{r().error}</pre>
                    </Show>
                </div>
            )}
        </Show>
    );
};

// ── Run panel ─────────────────────────────────────────────────────────

const RunPanel = (p: { model: WorkflowsViewModel }): JSX.Element => {
    const m = p.model;
    return (
        <footer class="workflows-runpanel">
            <div class="workflows-runpanel-title">
                Runs
                <Show when={m.activeRunIdAtom()}>
                    <span class="workflows-runpanel-active">
                        active: {m.activeRunIdAtom()?.slice(0, 8)}
                    </span>
                </Show>
            </div>
            <div class="workflows-runpanel-list">
                <Show
                    when={m.runsAtom().length > 0}
                    fallback={<div class="workflows-runpanel-empty">No runs yet.</div>}
                >
                    <For each={m.runsAtom()}>
                        {(r) => (
                            <div
                                class="workflows-runpanel-row"
                                classList={{
                                    "workflows-runpanel-row--ok": r.status === "done",
                                    "workflows-runpanel-row--err": r.status === "failed",
                                }}
                            >
                                <span class="workflows-runpanel-status">{r.status}</span>
                                <span class="workflows-runpanel-id">{r.id.slice(0, 8)}</span>
                                <span class="workflows-runpanel-output">
                                    {r.error ? r.error : r.output}
                                </span>
                            </div>
                        )}
                    </For>
                </Show>
            </div>
        </footer>
    );
};
