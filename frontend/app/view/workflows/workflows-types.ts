// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Workflows pane types (issue #753).
//
// The wire types (`WorkflowDefinition`, `WorkflowRun`, `WorkflowFlowNode`,
// `WorkflowFlowEdge`, `WorkflowBlockState`) live in the AgentMux global
// namespace at `frontend/types/gotypes.d.ts` so they're available to
// `rpc-api.ts` without imports. This file re-exports + narrows them
// for ergonomic use inside the workflows view.

export type BlockKind = "agent" | "condition" | "api" | "response" | "variables";

export type FlowNode = WorkflowFlowNode;
export type FlowEdge = WorkflowFlowEdge;
export type RunStatus = "running" | "done" | "failed";

export type WorkflowGraph = WorkflowDefinition["graph"];
export type WorkflowViewport = WorkflowDefinition["viewport"];

// Re-export the globals via type aliases so callers in this view can
// `import type { WorkflowDefinition } from "./workflows-types"` without
// reaching for the global directly.
export type { WorkflowDefinition, WorkflowRun, WorkflowBlockState as BlockState };

export const emptyGraph = (): WorkflowGraph => ({ nodes: [], edges: [] });
export const defaultViewport = (): WorkflowViewport => ({ x: 0, y: 0, zoom: 1 });
