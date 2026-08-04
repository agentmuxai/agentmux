// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Drone pane types (issue #753).
//
// The wire types (`DroneDefinition`, `DroneRun`, `DroneFlowNode`,
// `DroneFlowEdge`, `DroneBlockState`) live in the AgentMux global
// namespace at `frontend/types/gotypes.d.ts` so they're available to
// `rpc-api.ts` without imports. This file re-exports + narrows them
// for ergonomic use inside the drone view.

export type BlockKind = "agent" | "condition" | "api" | "response" | "variables";

export type FlowNode = DroneFlowNode;
export type FlowEdge = DroneFlowEdge;

export type DroneGraph = DroneDefinition["graph"];
export type DroneViewport = DroneDefinition["viewport"];

// Re-export the globals via type aliases so callers in this view can
// `import type { DroneDefinition } from "./drone-types"` without
// reaching for the global directly.
export type { DroneDefinition, DroneRun };

export const emptyGraph = (): DroneGraph => ({ nodes: [], edges: [] });
export const defaultViewport = (): DroneViewport => ({ x: 0, y: 0, zoom: 1 });
