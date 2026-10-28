// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { Accessor } from "solid-js";
import type { DocumentNode, MarkdownNode } from "./types";

type Dispatch = (action: { type: "StreamFlush"; newNodes: DocumentNode[]; updatedNodes: DocumentNode[] }) => void;

/**
 * Posts a permanent, visible line into the pane's own conversation, distinct from `log()`, which routes to the
 * hidden activity-log channel (docs/specs/SPEC_AGENT_PANE_AUTH_NOTIFICATIONS_2026_07_26.md §1). "success" and
 * "warning" get a symbol prefix; "info" doesn't (neutral narration like "Signing in..." or "Ready...").
 * With `replaces`, the line named by `id` is updated in place when it is still in the pane, else posted again.
 */
export function createSystemNotifier(document: Accessor<DocumentNode[]>, dispatch: Dispatch) {
    return (
        text: string,
        style: "info" | "warning" | "success" = "info",
        id = `system_notification_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
        replaces = false,
    ): void => {
        const prefix = style === "success" ? "✓ " : style === "warning" ? "⚠ " : "";
        const node = { type: "markdown", id, content: `${prefix}${text}` } as MarkdownNode;
        const inPane = replaces && document().some((n) => n.id === id);
        dispatch(inPane ? { type: "StreamFlush", newNodes: [], updatedNodes: [node] } : { type: "StreamFlush", newNodes: [node], updatedNodes: [] });
    };
}
