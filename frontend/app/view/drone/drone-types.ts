// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ── Trigger types ────────────────────────────────────────────────────────────

export type CronTrigger = {
    type: "cron";
    expr: string;       // standard 5-field cron expression
    timezone?: string;  // IANA tz, defaults to local
};

export type EventTrigger = {
    type: "event";
    eventName: string;  // e.g. "deploy.failed", "pr.opened"
    filter?: string;    // JSONPath filter on event payload
};

export type DependencyTrigger = {
    type: "dependency";
    droneId: string;
    on: "success" | "failure" | "any";
};

export type ManualTrigger = {
    type: "manual";
};

export type DroneTrigger = CronTrigger | EventTrigger | DependencyTrigger | ManualTrigger;

// ── Run state ────────────────────────────────────────────────────────────────

export type DroneRunState = "queued" | "running" | "success" | "failed" | "retrying" | "cancelled" | "timed_out";

// ── Drone definition ─────────────────────────────────────────────────────────

export interface DroneDefinition {
    id: string;
    name: string;
    description?: string;
    icon?: string;
    enabled: boolean;

    // Agent source — Forge agent id or inline prompt
    forgeAgentId?: string;
    inlineProvider?: string;  // "claude" | "codex" | "gemini"

    // The prompt/instruction sent on each run. Supports {{date}}, {{trigger_type}}, {{run_number}}
    task: string;

    triggers: DroneTrigger[];

    runPolicy: {
        retryCount: number;      // 0 = no retry
        retryDelaySecs: number;
        timeoutMins: number;     // 0 = no timeout
        maxConcurrent: number;   // 1 = no parallel runs of same drone
    };

    // Canvas position
    canvasX: number;
    canvasY: number;
}

// ── Run record ───────────────────────────────────────────────────────────────

export interface DroneRun {
    id: string;
    droneId: string;
    runNumber: number;
    triggerType: DroneTrigger["type"];
    triggerSource?: string;
    attempt: number;
    maxAttempts: number;
    state: DroneRunState;
    startedAt: number;   // unix ms
    endedAt?: number;
    outputSummary?: string;
    errorMsg?: string;
}

// ── Canvas state ─────────────────────────────────────────────────────────────

export interface CanvasTransform {
    x: number;
    y: number;
    scale: number;
}

// ── Default factory ──────────────────────────────────────────────────────────

export function makeDrone(partial?: Partial<DroneDefinition>): DroneDefinition {
    return {
        id: crypto.randomUUID(),
        name: "New Drone",
        enabled: true,
        task: "Run your automated task here.",
        triggers: [{ type: "manual" }],
        runPolicy: {
            retryCount: 0,
            retryDelaySecs: 30,
            timeoutMins: 30,
            maxConcurrent: 1,
        },
        canvasX: 100 + Math.random() * 200,
        canvasY: 100 + Math.random() * 200,
        ...partial,
    };
}
