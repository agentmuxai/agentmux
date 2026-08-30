// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createResource, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { abbreviateText } from "@/util/format-text";
import { BLOCK_KINDS, blockMeta } from "./block-registry";
import type { DroneViewModel } from "./drone-model";
import type { BlockKind, FlowNode } from "./drone-types";
import "./drone-view.scss";

// Transient global: which node-kind is being dragged from the top bar.
// Read by the Canvas's drop handler. Drag is inherently app-global, so a
// module-level signal is fine even with multiple drone panes open.
const [dragKind, setDragKind] = createSignal<BlockKind | null>(null);

export const DroneView = (props: { model: DroneViewModel }): JSX.Element => {
    const m = props.model;

    return (
        <div class="drone-pane">
            <NodeTypeBar model={m} />
            <div class="drone-stage">
                <Canvas model={m} />
                <RunPanel model={m} />
            </div>
        </div>
    );
};

DroneView.displayName = "DroneView";

// ── Top node-type bar ─────────────────────────────────────────────────
//
// The only permanent chrome. Left: drone name + open/new/save/run.
// Center: the horizontal row of draggable emoji node-type chips.
// Everything below this bar is canvas.

const NodeTypeBar = (p: { model: DroneViewModel }): JSX.Element => {
    const m = p.model;
    const validation = () => m.validate();
    return (
        <header class="drone-bar">
            <input
                class="drone-bar-name"
                value={m.draftAtom().name}
                onInput={(e) => m.setName(e.currentTarget.value)}
                onContextMenu={showTextInputContextMenu}
                placeholder="Untitled Drone"
                aria-label="Drone name"
            />

            <div class="drone-bar-types" role="toolbar" aria-label="Node types">
                <For each={BLOCK_KINDS}>{(kind) => <NodeChip model={m} kind={kind} />}</For>
            </div>

            <div class="drone-bar-actions">
                <Show when={m.listAtom().length > 0}>
                    <select
                        class="drone-btn"
                        aria-label="Open saved drone"
                        onChange={(e) => {
                            const id = e.currentTarget.value;
                            if (id) void m.openDrone(id);
                            e.currentTarget.value = "";
                        }}
                    >
                        <option value="">Open…</option>
                        <For each={m.listAtom()}>
                            {(wf) => <option value={wf.id}>{wf.name}</option>}
                        </For>
                    </select>
                </Show>
                <button class="drone-btn" onClick={() => m.newDrone()}>
                    New
                </button>
                <button class="drone-btn" onClick={() => void m.save()}>
                    Save
                </button>
                <button
                    class="drone-btn drone-btn--primary"
                    disabled={!validation().ok || m.runningAtom()}
                    title={validation().errors.join(" · ")}
                    onClick={() => void m.run()}
                >
                    {m.runningAtom() ? "Running…" : "▶ Run"}
                </button>
            </div>

            <Show when={m.errorAtom()}>
                <div class="drone-error" role="alert">
                    {m.errorAtom()}
                </div>
            </Show>
        </header>
    );
};

const NodeChip = (p: { model: DroneViewModel; kind: BlockKind }): JSX.Element => {
    const meta = blockMeta(p.kind);
    return (
        <button
            class="drone-chip"
            style={{ "--block-color": meta.color }}
            draggable={true}
            title={meta.description}
            aria-label={`${meta.label} node — drag onto the canvas, or click to add`}
            onDragStart={(e) => {
                setDragKind(p.kind);
                if (e.dataTransfer) {
                    e.dataTransfer.effectAllowed = "copy";
                    // Fallback channel in case the signal is cleared.
                    e.dataTransfer.setData("application/x-drone-kind", p.kind);
                }
            }}
            onDragEnd={() => setDragKind(null)}
            onClick={() => p.model.addNodeAtCenter(p.kind)}
        >
            <span class="drone-chip-emoji">{meta.emoji}</span>
            <span class="drone-chip-label">{meta.label}</span>
        </button>
    );
};

// ── Canvas ────────────────────────────────────────────────────────────
//
// A custom SolidJS SVG/HTML canvas (NOT a flow library). There is no
// production-grade SolidJS flow lib — React Flow is React-only and the one
// Solid port is a single-maintainer alpha — and the model is already
// xyflow-shaped, so we own the canvas and borrow only math where it pays
// off. The viewport transform + pointer drag are hand-rolled (spec §3.1).
// Supports: drag-from-bar to create, node drag, pan/zoom + fit, and
// port-to-port drag wiring. See SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05.md.

const Canvas = (p: { model: DroneViewModel }): JSX.Element => {
    const m = p.model;
    let canvasEl!: HTMLDivElement;

    // ── viewport transform (pan / zoom) ─────────────────────────────
    const vpStyle = () => {
        const v = m.viewportAtom();
        return {
            transform: `translate(${v.x}px, ${v.y}px) scale(${v.zoom})`,
            "transform-origin": "0 0",
        };
    };
    const ZMIN = 0.2,
        ZMAX = 2.5;
    const clampZoom = (z: number) => Math.min(ZMAX, Math.max(ZMIN, z));

    const onWheel = (e: WheelEvent) => {
        // Let scrollable areas inside nodes (class "nowheel") scroll
        // their content instead of zooming the canvas.
        if ((e.target as HTMLElement | null)?.closest(".nowheel")) return;
        e.preventDefault();
        const rect = canvasEl.getBoundingClientRect();
        const mx = e.clientX - rect.left;
        const my = e.clientY - rect.top;
        const v = m.viewportAtom();
        const zoom = clampZoom(v.zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1));
        // Anchor the flow-point under the cursor so zoom feels natural.
        const fx = (mx - v.x) / v.zoom;
        const fy = (my - v.y) / v.zoom;
        m.setViewport({ zoom, x: mx - fx * zoom, y: my - fy * zoom });
    };

    const zoomByCenter = (factor: number) => {
        const rect = canvasEl.getBoundingClientRect();
        const cx = rect.width / 2,
            cy = rect.height / 2;
        const v = m.viewportAtom();
        const zoom = clampZoom(v.zoom * factor);
        const fx = (cx - v.x) / v.zoom;
        const fy = (cy - v.y) / v.zoom;
        m.setViewport({ zoom, x: cx - fx * zoom, y: cy - fy * zoom });
    };

    const fitView = () => {
        const nodes = m.draftAtom().graph.nodes;
        const rect = canvasEl.getBoundingClientRect();
        if (nodes.length === 0) {
            m.setViewport({ x: 0, y: 0, zoom: 1 });
            return;
        }
        const NW = NODE_W,
            NH = 76,
            pad = 80;
        let minX = Infinity,
            minY = Infinity,
            maxX = -Infinity,
            maxY = -Infinity;
        for (const n of nodes) {
            minX = Math.min(minX, n.position.x);
            minY = Math.min(minY, n.position.y);
            maxX = Math.max(maxX, n.position.x + NW);
            maxY = Math.max(maxY, n.position.y + NH);
        }
        const w = maxX - minX || 1,
            h = maxY - minY || 1;
        const zoom = clampZoom(Math.min((rect.width - pad) / w, (rect.height - pad) / h));
        m.setViewport({
            zoom,
            x: rect.width / 2 - (minX + w / 2) * zoom,
            y: rect.height / 2 - (minY + h / 2) * zoom,
        });
    };

    // ── node drag (zoom-aware: divide client delta by zoom) ─────────
    let drag: { id: string; sx: number; sy: number; ox: number; oy: number; zoom: number } | null =
        null;
    const onNodeMove = (e: PointerEvent) => {
        if (!drag) return;
        const dx = (e.clientX - drag.sx) / drag.zoom;
        const dy = (e.clientY - drag.sy) / drag.zoom;
        m.moveNode(drag.id, { x: drag.ox + dx, y: drag.oy + dy });
    };
    const onNodeUp = () => {
        drag = null;
        window.removeEventListener("pointermove", onNodeMove);
    };
    const onNodePointerDown = (e: PointerEvent, n: FlowNode) => {
        if (e.button !== 0) return; // left-button drag only
        const t = e.target as HTMLElement;
        if (t.closest(".nodrag") || t.closest(".drone-node-close")) return;
        e.stopPropagation(); // don't let the canvas start a pan
        m.setSelected(n.id);
        m.setSelectedEdge(null);
        drag = {
            id: n.id,
            sx: e.clientX,
            sy: e.clientY,
            ox: n.position.x,
            oy: n.position.y,
            zoom: m.viewportAtom().zoom,
        };
        window.addEventListener("pointermove", onNodeMove);
        window.addEventListener("pointerup", onNodeUp, { once: true });
    };

    // ── canvas pan (drag empty background or middle-mouse) ──────────
    let pan: { sx: number; sy: number; ox: number; oy: number; moved: boolean } | null = null;
    const onPanMove = (e: PointerEvent) => {
        if (!pan) return;
        const dx = e.clientX - pan.sx,
            dy = e.clientY - pan.sy;
        if (Math.abs(dx) > 2 || Math.abs(dy) > 2) pan.moved = true;
        m.setViewport({ x: pan.ox + dx, y: pan.oy + dy });
    };
    const onPanUp = () => {
        // A background press that never moved is a click → deselect.
        if (pan && !pan.moved) m.setSelected(null);
        pan = null;
        window.removeEventListener("pointermove", onPanMove);
    };
    const isBackground = (t: HTMLElement) =>
        t === canvasEl ||
        t.classList.contains("drone-viewport") ||
        t.classList.contains("drone-canvas-edges") ||
        t.tagName.toLowerCase() === "svg";
    const onCanvasPointerDown = (e: PointerEvent) => {
        const t = e.target as HTMLElement;
        if (e.button === 1 || (e.button === 0 && isBackground(t))) {
            const v = m.viewportAtom();
            pan = { sx: e.clientX, sy: e.clientY, ox: v.x, oy: v.y, moved: false };
            window.addEventListener("pointermove", onPanMove);
            window.addEventListener("pointerup", onPanUp, { once: true });
        }
    };

    onCleanup(() => {
        window.removeEventListener("pointermove", onNodeMove);
        window.removeEventListener("pointermove", onPanMove);
    });

    // ── drag-from-bar → drop-on-canvas ──────────────────────────────
    const screenToFlow = (clientX: number, clientY: number) => {
        const rect = canvasEl.getBoundingClientRect();
        const v = m.viewportAtom();
        return {
            x: (clientX - rect.left - v.x) / v.zoom,
            y: (clientY - rect.top - v.y) / v.zoom,
        };
    };
    const onDragOver = (e: DragEvent) => {
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    };
    const onDrop = (e: DragEvent) => {
        e.preventDefault();
        const kind =
            dragKind() ??
            (e.dataTransfer?.getData("application/x-drone-kind") as BlockKind | undefined) ??
            null;
        if (!kind) return;
        const p0 = screenToFlow(e.clientX, e.clientY);
        // Center the node body roughly under the cursor.
        m.addNode(kind, { x: p0.x - NODE_W / 2, y: p0.y - 20 });
        setDragKind(null);
    };

    // Keep the model's canvas pixel size current so a chip-click can
    // place a node at the visible center.
    onMount(() => {
        const sync = () => {
            const r = canvasEl.getBoundingClientRect();
            m.setCanvasSize({ w: r.width, h: r.height });
        };
        sync();
        const ro = new ResizeObserver(sync);
        ro.observe(canvasEl);
        // Focus the canvas on any pointer-down within it (capture phase, so
        // it runs even when a node/edge/port handler stops propagation) —
        // this is what scopes the Delete/Escape keydown to this pane.
        const focusOnDown = () => canvasEl.focus({ preventScroll: true });
        canvasEl.addEventListener("pointerdown", focusOnDown, true);
        onCleanup(() => {
            ro.disconnect();
            canvasEl.removeEventListener("pointerdown", focusOnDown, true);
        });
    });

    // ── port → port connection drag ─────────────────────────────────
    const [conn, setConn] = createSignal<
        { sourceId: string; sourceHandle: string; x1: number; y1: number; x2: number; y2: number } | null
    >(null);
    const onConnMove = (e: PointerEvent) => {
        const c = conn();
        if (!c) return;
        const p = screenToFlow(e.clientX, e.clientY);
        setConn({ ...c, x2: p.x, y2: p.y });
    };
    // Tear down a connection drag — removes BOTH window listeners (the
    // pointerup is registered `once`, but an Escape-cancel resolves before
    // it fires, so we must remove it explicitly) and clears the preview.
    const cancelConn = () => {
        window.removeEventListener("pointermove", onConnMove);
        window.removeEventListener("pointerup", onConnUp);
        setConn(null);
    };
    const onConnUp = (e: PointerEvent) => {
        const c = conn();
        cancelConn();
        if (!c) return;
        // Drop target = whatever input port is under the cursor.
        const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
        const port = el?.closest(".drone-port--in") as HTMLElement | null;
        if (port?.dataset.node) {
            m.connect({
                source: c.sourceId,
                sourceHandle: c.sourceHandle,
                target: port.dataset.node,
                targetHandle: port.dataset.handle ?? "in",
            });
        }
    };
    const startConn = (e: PointerEvent, nodeId: string, handleId: string) => {
        if (e.button !== 0) return;
        e.stopPropagation(); // don't start a node drag
        const node = m.draftAtom().graph.nodes.find((n) => n.id === nodeId);
        if (!node) return;
        const sp = portFlowPos(node, "out", handleId);
        setConn({ sourceId: nodeId, sourceHandle: handleId, x1: sp.x, y1: sp.y, x2: sp.x, y2: sp.y });
        window.addEventListener("pointermove", onConnMove);
        window.addEventListener("pointerup", onConnUp, { once: true });
    };

    // ── keyboard: delete selection · escape cancels ─────────────────
    // Bound to the canvas element (not window) and the canvas is focused
    // on any pointer-down within it (capture phase, below), so Delete only
    // affects the pane the user is actually interacting with — never
    // another drone pane's selection in the same window.
    const onKey = (e: KeyboardEvent) => {
        const tag = (e.target as HTMLElement | null)?.tagName;
        if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
        if (e.key === "Escape") {
            cancelConn();
            m.setSelected(null);
            m.setSelectedEdge(null);
            return;
        }
        if (e.key === "Delete" || e.key === "Backspace") {
            const edgeId = m.selectedEdgeAtom();
            const nodeId = m.selectedAtom();
            if (edgeId) {
                m.removeEdge(edgeId);
                e.preventDefault();
            } else if (nodeId) {
                m.removeNode(nodeId);
                e.preventDefault();
            }
        }
    };
    onCleanup(() => {
        window.removeEventListener("pointermove", onConnMove);
        window.removeEventListener("pointerup", onConnUp);
    });

    return (
        <div
            class="drone-canvas"
            ref={canvasEl}
            tabindex={-1}
            onWheel={onWheel}
            onPointerDown={onCanvasPointerDown}
            onKeyDown={onKey}
            onDragOver={onDragOver}
            onDrop={onDrop}
        >
            <div class="drone-viewport" style={vpStyle()}>
                <svg class="drone-canvas-edges">
                    <For each={m.draftAtom().graph.edges}>
                        {(edge) => {
                            const src = () =>
                                m.draftAtom().graph.nodes.find((n) => n.id === edge.source);
                            const dst = () =>
                                m.draftAtom().graph.nodes.find((n) => n.id === edge.target);
                            const d = () => {
                                const a = portFlowPos(src()!, "out", edge.sourceHandle ?? "out");
                                const b = portFlowPos(dst()!, "in", edge.targetHandle ?? "in");
                                return bezierPath(a.x, a.y, b.x, b.y);
                            };
                            return (
                                <Show when={src() && dst()}>
                                    <path
                                        class="drone-edge-hit"
                                        d={d()}
                                        onPointerDown={(e) => {
                                            e.stopPropagation();
                                            m.setSelectedEdge(edge.id);
                                        }}
                                    />
                                    <path
                                        class="drone-edge"
                                        classList={{
                                            "drone-edge--selected": m.selectedEdgeAtom() === edge.id,
                                        }}
                                        d={d()}
                                    />
                                </Show>
                            );
                        }}
                    </For>
                    <Show when={conn()}>
                        {(c) => (
                            <path
                                class="drone-edge-preview"
                                d={bezierPath(c().x1, c().y1, c().x2, c().y2)}
                            />
                        )}
                    </Show>
                </svg>
                <For each={m.draftAtom().graph.nodes}>
                    {(n) => {
                        const meta = blockMeta(n.data.kind as BlockKind);
                        const selected = () => m.selectedAtom() === n.id;
                        return (
                            <div
                                class="drone-node"
                                classList={{
                                    "drone-node--selected": selected(),
                                }}
                                style={{
                                    left: `${n.position.x}px`,
                                    top: `${n.position.y}px`,
                                    "--block-color": meta.color,
                                }}
                                onPointerDown={(e) => onNodePointerDown(e, n)}
                            >
                                <header class="drone-node-header">
                                    <span class="drone-node-emoji">{meta.emoji}</span>
                                    <span class="drone-node-label">{meta.label}</span>
                                    <button
                                        class="drone-node-close"
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            m.removeNode(n.id);
                                        }}
                                        aria-label="Remove block"
                                    >
                                        ×
                                    </button>
                                </header>
                                <Show
                                    when={selected()}
                                    fallback={
                                        <div class="drone-node-summary">
                                            {nodeSummary(n)}
                                        </div>
                                    }
                                >
                                    <NodeFields
                                        model={m}
                                        node={n}
                                    />
                                </Show>
                                <For each={meta.inputs}>
                                    {(h, i) => (
                                        <div
                                            class="drone-port drone-port--in nodrag"
                                            data-node={n.id}
                                            data-handle={h.id}
                                            title={`input: ${h.label}`}
                                            style={{ top: `${portOffsetY(meta.inputs.length, i()) - 6}px` }}
                                        />
                                    )}
                                </For>
                                <For each={meta.outputs}>
                                    {(h, i) => (
                                        <div
                                            class="drone-port drone-port--out nodrag"
                                            data-node={n.id}
                                            data-handle={h.id}
                                            title={`output: ${h.label}`}
                                            style={{
                                                top: `${portOffsetY(meta.outputs.length, i()) - 6}px`,
                                                background:
                                                    h.id === "true"
                                                        ? "#10b981"
                                                        : h.id === "false"
                                                          ? "#6b7280"
                                                          : meta.color,
                                            }}
                                            onPointerDown={(e) => startConn(e, n.id, h.id)}
                                        />
                                    )}
                                </For>
                            </div>
                        );
                    }}
                </For>
            </div>
            <Show when={m.draftAtom().graph.nodes.length === 0}>
                <div class="drone-canvas-hint">
                    Drag a block from the top bar onto the canvas · drag nodes to move ·
                    drag a right port → a left port to wire · scroll to zoom · drag the
                    canvas to pan · Del removes the selection
                </div>
            </Show>
            <div class="drone-controls">
                <button
                    class="drone-ctrl"
                    title="Zoom in"
                    onClick={() => zoomByCenter(1.2)}
                >
                    +
                </button>
                <button
                    class="drone-ctrl"
                    title="Zoom out"
                    onClick={() => zoomByCenter(1 / 1.2)}
                >
                    −
                </button>
                <button class="drone-ctrl" title="Fit view" onClick={fitView}>
                    ⤢
                </button>
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

const truncate = (s: string, max = 40): string => abbreviateText(s, max);

// ── Port + edge geometry ──────────────────────────────────────────────
// Node body is NODE_W wide; ports sit on the left (inputs) and right
// (outputs) edges, distributed vertically around the header. These
// helpers are the single source of truth for where a wire attaches, used
// by both the rendered ports and the edge paths so they always line up.

const NODE_W = 248;

/** Vertical offset (px, node-relative) of port `idx` of `count`. */
function portOffsetY(count: number, idx: number): number {
    return 26 + (idx - (count - 1) / 2) * 20;
}

/** Flow-space center of a node's port. */
function portFlowPos(node: FlowNode, side: "in" | "out", handleId: string): { x: number; y: number } {
    const meta = blockMeta(node.data.kind as BlockKind);
    const list = side === "out" ? meta.outputs : meta.inputs;
    const idx = Math.max(0, list.findIndex((h) => h.id === handleId));
    return {
        x: side === "out" ? node.position.x + NODE_W : node.position.x,
        y: node.position.y + portOffsetY(list.length, idx),
    };
}

/** Horizontal cubic bezier between two points (left→right flow). */
function bezierPath(x1: number, y1: number, x2: number, y2: number): string {
    const dx = Math.max(40, Math.abs(x2 - x1) * 0.5);
    return `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`;
}

// ── NodeFields — inline in-node parameter editing ────────────────────
//
// Rendered inside the node body when the node is selected. Replaces the
// old right-side InspectorPanel: no more slide-over. Every interactive
// element carries class "nodrag" so interacting with a field never drags
// the node or pans/zooms the canvas.
//
// Long text fields (task, body, template) use auto-grow textareas capped
// at 5 lines. The Variables list stays inline here in PR1; a popover
// escalation for complex fields lands in PR2.

const NodeFields = (p: { model: DroneViewModel; node: FlowNode }): JSX.Element => {
    const update = (patch: Record<string, unknown>) =>
        p.model.updateNodeData(p.node.id, patch);

    return (
        <div class="drone-node-fields">
            <Show when={p.node.data.kind === "agent"}>
                <AgentRefEditor node={p.node} update={update} />
                <NodeField label="Task">
                    <textarea
                        class="drone-input drone-input--grow nodrag nowheel"
                        ref={(el) => autoGrow(el)}
                        value={(p.node.data["task"] as string) ?? ""}
                        placeholder="{{...}} interpolation supported"
                        onInput={(e) => {
                            update({ task: e.currentTarget.value });
                            autoGrow(e.currentTarget);
                        }}
                        onFocus={(e) => autoGrow(e.currentTarget)}
                        onContextMenu={showTextInputContextMenu}
                    />
                </NodeField>
                <AgentResultPanel model={p.model} blockId={p.node.id} />
            </Show>
            <Show when={p.node.data.kind === "api"}>
                <NodeField label="Method">
                    <select
                        class="drone-input nodrag"
                        value={(p.node.data["method"] as string) ?? "GET"}
                        onChange={(e) => update({ method: e.currentTarget.value })}
                    >
                        <option>GET</option>
                        <option>POST</option>
                        <option>PUT</option>
                        <option>PATCH</option>
                        <option>DELETE</option>
                    </select>
                </NodeField>
                <NodeField label="URL">
                    <input
                        class="drone-input nodrag"
                        value={(p.node.data["url"] as string) ?? ""}
                        onInput={(e) => update({ url: e.currentTarget.value })}
                        onContextMenu={showTextInputContextMenu}
                        placeholder="https://…/{{var.path}}"
                    />
                </NodeField>
                <NodeField label="Body">
                    <textarea
                        class="drone-input drone-input--grow nodrag nowheel"
                        ref={(el) => autoGrow(el)}
                        value={(p.node.data["body"] as string) ?? ""}
                        placeholder='{"key": "{{var.x}}"}'
                        onInput={(e) => {
                            update({ body: e.currentTarget.value });
                            autoGrow(e.currentTarget);
                        }}
                        onFocus={(e) => autoGrow(e.currentTarget)}
                        onContextMenu={showTextInputContextMenu}
                    />
                </NodeField>
            </Show>
            <Show when={p.node.data.kind === "condition"}>
                <NodeField label="if">
                    <input
                        class="drone-input nodrag"
                        value={(p.node.data["expr"] as string) ?? ""}
                        onInput={(e) => update({ expr: e.currentTarget.value })}
                        onContextMenu={showTextInputContextMenu}
                        placeholder="{{var.count}} > 0"
                    />
                </NodeField>
            </Show>
            <Show when={p.node.data.kind === "response"}>
                <NodeField label="Template">
                    <textarea
                        class="drone-input drone-input--grow nodrag nowheel"
                        ref={(el) => autoGrow(el)}
                        value={(p.node.data["template"] as string) ?? ""}
                        placeholder="Hello {{var.name}}!"
                        onInput={(e) => {
                            update({ template: e.currentTarget.value });
                            autoGrow(e.currentTarget);
                        }}
                        onFocus={(e) => autoGrow(e.currentTarget)}
                        onContextMenu={showTextInputContextMenu}
                    />
                </NodeField>
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

/** Compact label + control row rendered inside a node. */
const NodeField = (p: { label: string; children: JSX.Element }): JSX.Element => (
    <label class="drone-node-field">
        <span class="drone-node-field-label">{p.label}</span>
        {p.children}
    </label>
);

/** Auto-grow a textarea up to a CSS max-height cap. */
function autoGrow(el: HTMLTextAreaElement): void {
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
}


const VariablesEditor = (p: {
    entries: Array<{ name: string; value: string }>;
    onChange: (next: Array<{ name: string; value: string }>) => void;
}): JSX.Element => {
    const update = (i: number, patch: Partial<{ name: string; value: string }>) => {
        p.onChange(p.entries.map((e, idx) => (idx === i ? { ...e, ...patch } : e)));
    };
    return (
        <div class="drone-vars">
            <For each={p.entries}>
                {(entry, i) => (
                    <div class="drone-vars-row">
                        <input
                            class="drone-input nodrag"
                            value={entry.name}
                            onInput={(e) => update(i(), { name: e.currentTarget.value })}
                            onContextMenu={showTextInputContextMenu}
                            placeholder="name"
                        />
                        <input
                            class="drone-input nodrag"
                            value={entry.value}
                            onInput={(e) => update(i(), { value: e.currentTarget.value })}
                            onContextMenu={showTextInputContextMenu}
                            placeholder="value"
                        />
                        <button
                            class="drone-btn drone-btn--small nodrag"
                            onClick={() => p.onChange(p.entries.filter((_, idx) => idx !== i()))}
                        >
                            ×
                        </button>
                    </div>
                )}
            </For>
            <button
                class="drone-btn drone-btn--small nodrag"
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
// `forge_agent_id` text field with Memory / instance-name pickers backed
// by the launch-modal RPCs. The Identity picker that originally shipped
// here was removed in Phase 4b of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md
// — the drone runner never read `agent_ref.identityId` for credential
// injection, so the control was already inert; `identityId` is still
// carried in `AgentRefShape`/`readAgentRef` below but has no picker UI.

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
            `[drone] Agent block ${n.id} uses legacy forge_agent_id="${legacy}"; re-pick identity/memory after PR 3.`,
        );
    }
    return { identityId: "", memoryId: "", instanceName: "", workingDirectory: "" };
}

const AgentRefEditor = (p: {
    node: FlowNode;
    update: (patch: Record<string, unknown>) => void;
}): JSX.Element => {
    const [memories] = createResource(() =>
        RpcApi.ListMemoriesCommand(TabRpcClient, {}).catch(() => [] as Memory[]),
    );
    const ref = () => readAgentRef(p.node);
    const setRef = (patch: Partial<AgentRefShape>) =>
        p.update({ agent_ref: { ...ref(), ...patch } });

    return (
        <>
            <NodeField label="Memory">
                <select
                    class="drone-input nodrag"
                    value={ref().memoryId}
                    onChange={(e) => setRef({ memoryId: e.currentTarget.value })}
                >
                    <option value="">— blank —</option>
                    {/* is_system entries are AgentMux-controlled workspace policy,
                        not a selectable per-agent bundle (reagent P1, PR #2782). */}
                    <For each={(memories() ?? []).filter((m) => !m.is_blank && !m.is_system)}>
                        {(memory) => <option value={memory.id}>{memory.name}</option>}
                    </For>
                </select>
            </NodeField>
            <NodeField label="Instance">
                <input
                    class="drone-input nodrag"
                    value={ref().instanceName}
                    onInput={(e) => setRef({ instanceName: e.currentTarget.value })}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="blank = one-shot"
                />
            </NodeField>
        </>
    );
};

// ── Agent result panel ────────────────────────────────────────────────
//
// Shows the most recent run's BlockDone output for the selected Agent
// block. Subscribed via the model (`dronerun:<id>` events → §5.2).
// Phase 1.5 ships final-result rendering only; hover-expand tool stream
// is deferred to Phase 2 polish.

const AgentResultPanel = (p: {
    model: DroneViewModel;
    blockId: string;
}): JSX.Element => {
    const result = () => p.model.blockResultAtom(p.blockId);
    return (
        <Show when={result()}>
            {(r) => (
                <div class="drone-agent-result">
                    <div class="drone-agent-result-label">Last run</div>
                    <Show
                        when={r().error}
                        fallback={
                            <>
                                <pre class="drone-agent-result-text">{r().response}</pre>
                                <Show when={r().costUsd != null}>
                                    <div class="drone-agent-result-cost">
                                        ${r().costUsd!.toFixed(4)}
                                    </div>
                                </Show>
                            </>
                        }
                    >
                        <pre class="drone-agent-result-error">{r().error}</pre>
                    </Show>
                </div>
            )}
        </Show>
    );
};

// ── Run panel ─────────────────────────────────────────────────────────

const RunPanel = (p: { model: DroneViewModel }): JSX.Element => {
    const m = p.model;
    // Bottom overlay drawer — only present when there's something to
    // show, so the empty canvas stays clean.
    return (
        <Show when={m.runsAtom().length > 0 || m.activeRunIdAtom()}>
            <footer class="drone-runpanel">
                <div class="drone-runpanel-title">
                    Runs
                    <Show when={m.activeRunIdAtom()}>
                        <span class="drone-runpanel-active">
                            active: {m.activeRunIdAtom()?.slice(0, 8)}
                        </span>
                    </Show>
                </div>
                <div class="drone-runpanel-list">
                    <Show
                        when={m.runsAtom().length > 0}
                        fallback={<div class="drone-runpanel-empty">No runs yet.</div>}
                    >
                        <For each={m.runsAtom()}>
                            {(r) => (
                                <div
                                    class="drone-runpanel-row"
                                    classList={{
                                        "drone-runpanel-row--ok": r.status === "done",
                                        "drone-runpanel-row--err": r.status === "failed",
                                    }}
                                >
                                    <span class="drone-runpanel-status">{r.status}</span>
                                    <span class="drone-runpanel-id">{r.id.slice(0, 8)}</span>
                                    <span class="drone-runpanel-output">
                                        {r.error ? r.error : r.output}
                                    </span>
                                </div>
                            )}
                        </For>
                    </Show>
                </div>
            </footer>
        </Show>
    );
};
