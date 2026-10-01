// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tool row's header text, composed from the node's own fields at render
 * time instead of the `summary` string baked at parse time.
 *
 * The baked string repeated what ToolBlock already renders around it (a
 * second status glyph, a second duration), went stale when the reducer changed
 * a status without re-parsing (a canceled tool still read ⏳), and showed MCP
 * tools by their raw `mcp__server__tool` name.
 * SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.2.
 */

import { formatReadRangeShort, readRangeOf } from "../tool-meta/read-range";
import { toolDetailOf, toolIcon, toolLabel, toolNameOf } from "../tool-meta/tool-descriptors";
import type { ToolNode } from "../types";

export interface ToolHeaderParts {
    icon: string;
    /** Tool name as shown; null when the icon and detail already say it. */
    label: string | null;
    /** The tool's main argument (path, command, query, …); "" when none. */
    detail: string;
    /** Which part of the file a Read covers ("L120–179 of 456"); null for any other tool. */
    range: string | null;
}

/** Icon, label and detail from the tool's descriptor (tool-meta/tool-descriptors.ts). */
export function toolHeaderParts(node: ToolNode): ToolHeaderParts {
    const detail = toolDetailOf(node);
    const range = readRangeOf(node);
    return {
        icon: toolIcon(node),
        label: toolLabel(toolNameOf(node), detail),
        detail,
        range: range ? formatReadRangeShort(range) : null,
    };
}

/** AskUserQuestion's flow writes its own row text into `summary` ("❓ Waiting
 *  for your answer", "❓ Answered — …"); for it, that text IS the header. */
export function hasAuthoredSummary(node: ToolNode): boolean {
    return node.toolName === "AskUserQuestion";
}

/** The header as one plain string for text consumers (the /btw snapshot),
 *  e.g. "🌐 solid docs", plus any reducer-set `statusNote` — which ToolBlock
 *  renders in its own span — so no consumer loses it. An authored summary
 *  (see `hasAuthoredSummary`) is returned as-is. */
export function toolHeaderText(node: ToolNode): string {
    if (hasAuthoredSummary(node)) return node.summary;
    const { icon, label, detail, range } = toolHeaderParts(node);
    const header = [icon, label, detail, range].filter(Boolean).join(" ");
    return node.statusNote ? `${header} — ${node.statusNote}` : header;
}
