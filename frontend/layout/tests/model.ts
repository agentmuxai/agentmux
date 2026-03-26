// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { LayoutNode, LayoutTreeState } from "../lib/types";

export function newLayoutTreeState(rootNode: LayoutNode): LayoutTreeState {
    return {
        rootNode,
        pendingBackendActions: [],
    };
}
