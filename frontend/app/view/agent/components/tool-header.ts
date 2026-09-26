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

import { extractToolDetail } from "../stream-parser";
import { TOOL_ICONS, type ToolNode } from "../types";

export interface ToolHeaderParts {
    icon: string;
    /** Tool name as shown; null when the icon and detail already say it. */
    label: string | null;
    /** The tool's main argument (path, command, query, …); "" when none. */
    detail: string;
}

/** Tools whose icon plus detail identify them without the name: a globe
 *  followed by a search query or a host/path can only be a web tool. */
const SELF_DESCRIBING = new Set(["WebSearch", "web_search", "WebFetch", "web_fetch"]);

const MCP_PREFIX = "mcp__";

/** "mcp__agentmux__WhoAmI" → "agentmux · WhoAmI"; null if not an MCP name. */
export function mcpDisplayName(name: string): string | null {
    if (!name.startsWith(MCP_PREFIX)) return null;
    const rest = name.slice(MCP_PREFIX.length);
    const sep = rest.indexOf("__");
    if (sep <= 0 || sep + 2 >= rest.length) return null;
    return `${rest.slice(0, sep)} · ${rest.slice(sep + 2)}`;
}

export function toolHeaderParts(node: ToolNode): ToolHeaderParts {
    const name = node.toolName ?? node.tool;
    const params = (node.params as Record<string, any>) ?? {};
    const icon = TOOL_ICONS[name] ?? TOOL_ICONS[node.tool] ?? TOOL_ICONS.Other;
    // The raw name first: a WebSearch's coarse kind is "Other", which has no
    // detail rule. The coarse kind covers provider aliases the raw name misses.
    const detail = extractToolDetail(name, params) || (name !== node.tool ? extractToolDetail(node.tool, params) : "");
    const label = SELF_DESCRIBING.has(name) && detail ? null : (mcpDisplayName(name) ?? name);
    return { icon, label, detail };
}

/** AskUserQuestion's flow writes its own row text into `summary` ("❓ Waiting
 *  for your answer", "❓ Answered — …"); for it, that text IS the header. */
export function hasAuthoredSummary(node: ToolNode): boolean {
    return node.toolName === "AskUserQuestion";
}

/** The same header as one plain string, e.g. "🌐 solid docs" (or the authored
 *  summary, see `hasAuthoredSummary`). */
export function toolHeaderText(node: ToolNode): string {
    if (hasAuthoredSummary(node)) return node.summary;
    const { icon, label, detail } = toolHeaderParts(node);
    return [icon, label, detail].filter(Boolean).join(" ");
}
