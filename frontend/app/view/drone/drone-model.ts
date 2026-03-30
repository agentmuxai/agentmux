// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, type Accessor, type Setter } from "solid-js";
import type { BlockNodeModel } from "@/app/block/blocktypes";
import type { DroneDefinition, DroneRun, CanvasTransform } from "./drone-types";
import { makeDrone } from "./drone-types";

export class DroneViewModel implements ViewModel {
    viewType = "drone";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "drone";
    viewName: Accessor<string> = () => "Drone";
    noPadding: Accessor<boolean> = () => true;

    get viewComponent(): ViewComponent {
        return null; // wired in drone.tsx barrel to avoid circular import
    }

    viewText: Accessor<string | HeaderElem[]>;

    // Drone list
    private _drones = createSignal<DroneDefinition[]>([]);
    dronesAtom: Accessor<DroneDefinition[]> = this._drones[0];
    private setDrones: Setter<DroneDefinition[]> = this._drones[1];

    // Latest run per drone (droneId → DroneRun)
    private _latestRuns = createSignal<Map<string, DroneRun>>(new Map());
    latestRunsAtom: Accessor<Map<string, DroneRun>> = this._latestRuns[0];
    private setLatestRuns: Setter<Map<string, DroneRun>> = this._latestRuns[1];

    // Run history for selected drone
    private _runHistory = createSignal<DroneRun[]>([]);
    runHistoryAtom: Accessor<DroneRun[]> = this._runHistory[0];
    private setRunHistory: Setter<DroneRun[]> = this._runHistory[1];

    // Selected drone for edit panel / run log
    private _selectedDroneId = createSignal<string | null>(null);
    selectedDroneIdAtom: Accessor<string | null> = this._selectedDroneId[0];
    private setSelectedDroneId: Setter<string | null> = this._selectedDroneId[1];

    // Edit panel open state
    private _editPanelOpen = createSignal<boolean>(false);
    editPanelOpenAtom: Accessor<boolean> = this._editPanelOpen[0];
    private setEditPanelOpen: Setter<boolean> = this._editPanelOpen[1];

    // Run log panel open state
    private _runLogOpen = createSignal<boolean>(false);
    runLogOpenAtom: Accessor<boolean> = this._runLogOpen[0];
    private setRunLogOpen: Setter<boolean> = this._runLogOpen[1];

    // Canvas transform (pan/zoom)
    private _canvasTransform = createSignal<CanvasTransform>({ x: 0, y: 0, scale: 1 });
    canvasTransformAtom: Accessor<CanvasTransform> = this._canvasTransform[0];
    private setCanvasTransform: Setter<CanvasTransform> = this._canvasTransform[1];

    // Drone being edited (copy, not the live one)
    private _editingDrone = createSignal<DroneDefinition | null>(null);
    editingDroneAtom: Accessor<DroneDefinition | null> = this._editingDrone[0];
    setEditingDrone: Setter<DroneDefinition | null> = this._editingDrone[1];

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.viewText = () => this._summaryText();
        // Seed with a demo drone so the canvas isn't empty on first open
        this.setDrones([
            makeDrone({
                name: "Morning Scan",
                description: "Daily market or status scan",
                triggers: [{ type: "cron", expr: "0 7 * * 1-5" }],
                canvasX: 80,
                canvasY: 120,
            }),
        ]);
    }

    private _summaryText(): string {
        const drones = this.dronesAtom();
        const runs = this.latestRunsAtom();
        const running = drones.filter((d) => runs.get(d.id)?.state === "running").length;
        const failed = drones.filter((d) => runs.get(d.id)?.state === "failed").length;
        if (running > 0) return `${running} running`;
        if (failed > 0) return `${failed} failed`;
        return `${drones.length} drone${drones.length !== 1 ? "s" : ""}`;
    }

    // ── Actions ───────────────────────────────────────────────────────────────

    selectDrone(id: string | null) {
        this.setSelectedDroneId(id);
        if (id) {
            // Load run history for this drone (mock for Phase 1)
            const runs = this._mockRunHistory(id);
            this.setRunHistory(runs);
        }
    }

    openEditPanel(droneId?: string) {
        if (droneId) {
            const drone = this.dronesAtom().find((d) => d.id === droneId);
            if (drone) this.setEditingDrone({ ...drone });
        } else {
            // New drone
            this.setEditingDrone(makeDrone());
        }
        this.setEditPanelOpen(true);
    }

    closeEditPanel() {
        this.setEditPanelOpen(false);
        this.setEditingDrone(null);
    }

    saveDrone(drone: DroneDefinition) {
        const existing = this.dronesAtom();
        const idx = existing.findIndex((d) => d.id === drone.id);
        if (idx >= 0) {
            const updated = [...existing];
            updated[idx] = drone;
            this.setDrones(updated);
        } else {
            this.setDrones([...existing, drone]);
        }
        this.closeEditPanel();
    }

    deleteDrone(droneId: string) {
        this.setDrones(this.dronesAtom().filter((d) => d.id !== droneId));
        if (this.selectedDroneIdAtom() === droneId) this.setSelectedDroneId(null);
    }

    toggleEnabled(droneId: string) {
        const updated = this.dronesAtom().map((d) =>
            d.id === droneId ? { ...d, enabled: !d.enabled } : d
        );
        this.setDrones(updated);
    }

    triggerDrone(droneId: string) {
        // Phase 1: simulate a run with mock state
        const drone = this.dronesAtom().find((d) => d.id === droneId);
        if (!drone || !drone.enabled) return;
        const run: DroneRun = {
            id: crypto.randomUUID(),
            droneId,
            runNumber: (this._mockRunHistory(droneId).length ?? 0) + 1,
            triggerType: "manual",
            triggerSource: "user",
            attempt: 1,
            maxAttempts: drone.runPolicy.retryCount + 1,
            state: "running",
            startedAt: Date.now(),
        };
        const map = new Map(this.latestRunsAtom());
        map.set(droneId, run);
        this.setLatestRuns(map);
        // Simulate completion after 3s
        setTimeout(() => {
            const updated = new Map(this.latestRunsAtom());
            updated.set(droneId, { ...run, state: "success", endedAt: Date.now() });
            this.setLatestRuns(updated);
        }, 3000);
    }

    moveDrone(droneId: string, x: number, y: number) {
        const updated = this.dronesAtom().map((d) =>
            d.id === droneId ? { ...d, canvasX: x, canvasY: y } : d
        );
        this.setDrones(updated);
    }

    updateCanvasTransform(t: Partial<CanvasTransform>) {
        this.setCanvasTransform({ ...this.canvasTransformAtom(), ...t });
    }

    toggleRunLog() {
        this.setRunLogOpen(!this.runLogOpenAtom());
    }

    selectedDrone(): DroneDefinition | null {
        const id = this.selectedDroneIdAtom();
        return this.dronesAtom().find((d) => d.id === id) ?? null;
    }

    // ── Mock data (Phase 1 — no backend yet) ─────────────────────────────────

    private _mockRunHistory(droneId: string): DroneRun[] {
        const now = Date.now();
        return [
            {
                id: "r1",
                droneId,
                runNumber: 3,
                triggerType: "cron",
                attempt: 1,
                maxAttempts: 1,
                state: "success",
                startedAt: now - 2 * 60 * 60 * 1000,
                endedAt: now - 2 * 60 * 60 * 1000 + 192000,
            },
            {
                id: "r2",
                droneId,
                runNumber: 2,
                triggerType: "cron",
                attempt: 1,
                maxAttempts: 1,
                state: "success",
                startedAt: now - 26 * 60 * 60 * 1000,
                endedAt: now - 26 * 60 * 60 * 1000 + 175000,
            },
            {
                id: "r3",
                droneId,
                runNumber: 1,
                triggerType: "manual",
                triggerSource: "user",
                attempt: 1,
                maxAttempts: 1,
                state: "failed",
                startedAt: now - 50 * 60 * 60 * 1000,
                endedAt: now - 50 * 60 * 60 * 1000 + 64000,
                errorMsg: "Agent exited with code 1",
            },
        ];
    }

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {}
}
