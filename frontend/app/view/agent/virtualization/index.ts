// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Public surface of the agent-pane virtualization redesign foundation
 * (Phase 1). Phase 2 adds the view-layer component; Phase 3 adds the
 * perf-probe HUD; both will extend exports from here.
 *
 * See docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md.
 */

export {
    captureTopmostAnchor,
    isNearBottom,
    isNearTop,
    NEAR_TOP_THRESHOLD_PX,
    restoreScrollFromAnchor,
    STICK_TO_BOTTOM_THRESHOLD_PX,
    type ScrollAnchor,
} from "./anchor";

export {
    createAgentViewState,
    type AgentViewState,
} from "./state";

export {
    locateIndex,
    partitionForVirtualization,
    STREAMING_BUFFER_SIZE,
    type VirtualizationPartition,
} from "./streaming-buffer";

export {
    buildRendererRegistry,
    estimateAgentMessage,
    estimateMarkdown,
    estimateNode,
    estimateSection,
    estimateTextHeight,
    estimateTool,
    estimateUserMessage,
    STREAMING_CAPABLE,
    type NodeKind,
    type NodeKindRenderer,
    type NodeOf,
    type NodeRendererRegistry,
    type RendererComponents,
} from "./renderers";
