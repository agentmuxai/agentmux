// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which tools preview "content-first": once finished, the result shows
 * expanded by default — in history too — like a message rather than a
 * collapsible tool call; the user can still collapse it (collapsedNodes).
 * SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.1.
 *
 * A plain name table rather than a field on the renderer registry: the
 * virtualizer (expansion-source.ts, renderers.ts) needs this answer too, and
 * the registry is only populated as a side effect of importing
 * ToolOverlayLog, which those modules — and their tests — never do.
 */

import type { ToolNode } from "../types";

const CONTENT_FIRST = new Set(["WebSearch", "web_search"]);

const isContentFirstName = (node: ToolNode): boolean => CONTENT_FIRST.has(node.toolName ?? node.tool);

/** Expanded by default: a content-first tool that finished with a result
 *  (success, or failed with its error). While running it's auto-expanded
 *  like any tool; denied/canceled have nothing to show. */
export function isContentFirstTool(node: ToolNode): boolean {
    return isContentFirstName(node) && (node.status === "success" || node.status === "failed");
}

/** Its preview box opens at the top (reading position), not following the
 *  latest output. By name, not status: the box is mounted while the tool is
 *  still running and must not start pinned to the bottom. */
export function startsAtTop(node: ToolNode): boolean {
    return isContentFirstName(node);
}
