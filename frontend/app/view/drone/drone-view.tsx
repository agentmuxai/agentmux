// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import type { DroneViewModel } from "./drone-model";
import type { CanvasTransform } from "./drone-types";
import { DroneNode } from "./drone-node";
import { DroneEditPanel } from "./drone-edit-panel";
import { DroneRunLog } from "./drone-run-log";
import "./drone-view.css";

export function DroneView(props: ViewComponentProps<DroneViewModel>): JSX.Element {
    const model = props.model;
    let canvasRef: HTMLDivElement | undefined;
    let draggingNodeId: string | null = null;
    let dragStartMouse = { x: 0, y: 0 };
    let dragStartPos = { x: 0, y: 0 };
    let isPanning = false;
    let panStart = { x: 0, y: 0 };
    let panStartTransform = { x: 0, y: 0 };

    // ── Canvas pan/zoom ──────────────────────────────────────────────────────

    function onCanvasWheel(e: WheelEvent) {
        e.preventDefault();
        const t = model.canvasTransformAtom();
        const delta = e.deltaY > 0 ? 0.9 : 1.1;
        const newScale = Math.max(0.3, Math.min(2.5, t.scale * delta));
        // Zoom toward cursor
        const rect = canvasRef!.getBoundingClientRect();
        const cx = e.clientX - rect.left;
        const cy = e.clientY - rect.top;
        const newX = cx - (cx - t.x) * (newScale / t.scale);
        const newY = cy - (cy - t.y) * (newScale / t.scale);
        model.updateCanvasTransform({ scale: newScale, x: newX, y: newY });
    }

    function onCanvasMouseDown(e: MouseEvent) {
        if (e.button !== 0) return;
        // Pan when clicking empty canvas (not a node)
        const target = e.target as HTMLElement;
        if (target === canvasRef || target.classList.contains("drone-canvas__world")) {
            isPanning = true;
            panStart = { x: e.clientX, y: e.clientY };
            panStartTransform = { x: model.canvasTransformAtom().x, y: model.canvasTransformAtom().y };
            model.selectDrone(null);
        }
    }

    function onMouseMove(e: MouseEvent) {
        if (isPanning) {
            const dx = e.clientX - panStart.x;
            const dy = e.clientY - panStart.y;
            model.updateCanvasTransform({ x: panStartTransform.x + dx, y: panStartTransform.y + dy });
        }
        if (draggingNodeId) {
            const t = model.canvasTransformAtom();
            const dx = (e.clientX - dragStartMouse.x) / t.scale;
            const dy = (e.clientY - dragStartMouse.y) / t.scale;
            model.moveDrone(draggingNodeId, dragStartPos.x + dx, dragStartPos.y + dy);
        }
    }

    function onMouseUp() {
        isPanning = false;
        draggingNodeId = null;
    }

    function fitToView() {
        const drones = model.dronesAtom();
        if (!drones.length || !canvasRef) return;
        const xs = drones.map((d) => d.canvasX);
        const ys = drones.map((d) => d.canvasY);
        const minX = Math.min(...xs) - 40;
        const minY = Math.min(...ys) - 40;
        const maxX = Math.max(...xs) + 200;
        const maxY = Math.max(...ys) + 100;
        const rect = canvasRef.getBoundingClientRect();
        const scaleX = rect.width / (maxX - minX);
        const scaleY = rect.height / (maxY - minY);
        const scale = Math.min(1, Math.max(0.3, Math.min(scaleX, scaleY) * 0.9));
        model.updateCanvasTransform({
            scale,
            x: (rect.width - (maxX + minX) * scale) / 2,
            y: (rect.height - (maxY + minY) * scale) / 2,
        });
    }

    onMount(() => {
        window.addEventListener("mousemove", onMouseMove);
        window.addEventListener("mouseup", onMouseUp);
    });
    onCleanup(() => {
        window.removeEventListener("mousemove", onMouseMove);
        window.removeEventListener("mouseup", onMouseUp);
    });

    // ── Node drag start ──────────────────────────────────────────────────────

    function startNodeDrag(e: MouseEvent, droneId: string) {
        e.stopPropagation();
        if (e.button !== 0) return;
        const drone = model.dronesAtom().find((d) => d.id === droneId);
        if (!drone) return;
        draggingNodeId = droneId;
        dragStartMouse = { x: e.clientX, y: e.clientY };
        dragStartPos = { x: drone.canvasX, y: drone.canvasY };
    }

    // ── World transform CSS ──────────────────────────────────────────────────

    const worldStyle = () => {
        const t = model.canvasTransformAtom();
        return `transform: translate(${t.x}px, ${t.y}px) scale(${t.scale}); transform-origin: 0 0;`;
    };

    return (
        <div class="drone-view">
            {/* Toolbar */}
            <div class="drone-toolbar">
                <span class="drone-toolbar__title">Drones</span>
                <div class="drone-toolbar__actions">
                    <button class="drone-toolbar__btn" onClick={fitToView} title="Fit to view (F)">
                        ⊞
                    </button>
                    <button
                        class="drone-toolbar__btn"
                        onClick={() => model.updateCanvasTransform({ scale: Math.min(2.5, model.canvasTransformAtom().scale * 1.2) })}
                        title="Zoom in"
                    >
                        +
                    </button>
                    <button
                        class="drone-toolbar__btn"
                        onClick={() => model.updateCanvasTransform({ scale: Math.max(0.3, model.canvasTransformAtom().scale * 0.8) })}
                        title="Zoom out"
                    >
                        −
                    </button>
                    <button
                        class="drone-toolbar__btn drone-toolbar__btn--primary"
                        onClick={() => model.openEditPanel()}
                        title="Add drone"
                    >
                        + Add Drone
                    </button>
                </div>
            </div>

            {/* Canvas area */}
            <div class="drone-canvas-wrap">
                <div
                    class="drone-canvas"
                    ref={canvasRef}
                    onWheel={onCanvasWheel}
                    onMouseDown={onCanvasMouseDown}
                >
                    <div class="drone-canvas__world" style={worldStyle()}>
                        {/* Dependency edges (SVG layer) */}
                        <svg class="drone-canvas__edges">
                            <For each={model.dronesAtom()}>
                                {(drone) => (
                                    <For each={drone.triggers.filter((t) => t.type === "dependency")}>
                                        {(trigger) => {
                                            if (trigger.type !== "dependency") return null;
                                            const src = model.dronesAtom().find((d) => d.id === trigger.droneId);
                                            if (!src) return null;
                                            const x1 = src.canvasX + 90;
                                            const y1 = src.canvasY + 56;
                                            const x2 = drone.canvasX + 90;
                                            const y2 = drone.canvasY;
                                            const my = (y1 + y2) / 2;
                                            return (
                                                <path
                                                    d={`M ${x1} ${y1} C ${x1} ${my}, ${x2} ${my}, ${x2} ${y2}`}
                                                    class={`drone-edge drone-edge--${trigger.on}`}
                                                    fill="none"
                                                />
                                            );
                                        }}
                                    </For>
                                )}
                            </For>
                        </svg>

                        {/* Nodes */}
                        <For each={model.dronesAtom()}>
                            {(drone) => (
                                <div
                                    class="drone-canvas__node-wrap"
                                    style={{ left: `${drone.canvasX}px`, top: `${drone.canvasY}px` }}
                                >
                                    <DroneNode
                                        drone={drone}
                                        latestRun={model.latestRunsAtom().get(drone.id)}
                                        model={model}
                                        selected={model.selectedDroneIdAtom() === drone.id}
                                        onDragStart={(e) => startNodeDrag(e, drone.id)}
                                    />
                                </div>
                            )}
                        </For>
                    </div>

                    {/* Empty state */}
                    <Show when={model.dronesAtom().length === 0}>
                        <div class="drone-canvas__empty">
                            <div class="drone-canvas__empty-icon">🤖</div>
                            <div class="drone-canvas__empty-title">No drones yet</div>
                            <div class="drone-canvas__empty-desc">
                                Drones are automated agents triggered by schedules or events.
                            </div>
                            <button
                                class="drone-canvas__empty-btn"
                                onClick={() => model.openEditPanel()}
                            >
                                + Add your first drone
                            </button>
                        </div>
                    </Show>
                </div>

                {/* Edit panel (right drawer) */}
                <Show when={model.editPanelOpenAtom()}>
                    <DroneEditPanel model={model} />
                </Show>
            </div>

            {/* Run log (bottom drawer) */}
            <DroneRunLog model={model} />
        </div>
    );
}
