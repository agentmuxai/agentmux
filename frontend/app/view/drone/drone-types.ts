// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Drone pane types (issue #753).
//
// The wire types (`DroneDefinition`, `DroneRun`, `DroneFlowNode`,
// `DroneFlowEdge`, `DroneBlockState`) are GENERATED from Rust by ts-rs and
// re-exported through `@/app/store/rpc-api`. They used to be hand-written
// ambient globals here; this file still re-exports + narrows them for
// ergonomic use inside the drone view.

import type {
    BlockKind,
    DroneDefinition,
    DroneFlowEdge,
    DroneFlowNode,
    DroneGraph,
    DroneRun,
    DroneViewport,
} from "@/app/store/rpc-api";

// `BlockKind` used to be a hand-written union here. It is the Rust
// `BlockKind` enum, so it is generated now and cannot drift from the kinds
// the executor actually dispatches on.
export type { BlockKind };

export type FlowNode = DroneFlowNode;
export type FlowEdge = DroneFlowEdge;

export type { DroneGraph, DroneViewport };

export type { DroneDefinition, DroneRun };

export const emptyGraph = (): DroneGraph => ({ nodes: [], edges: [] });
export const defaultViewport = (): DroneViewport => ({ x: 0, y: 0, zoom: 1 });
