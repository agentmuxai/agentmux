// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tool-result renderer registry — decouples "how a tool result renders" from a
 * hardcoded `switch` over the closed `ToolNode.tool` enum, so rich per-tool UIs
 * (terminal, diff, search-result cards, …) are drop-in and the open-ended tool
 * universe (web tools, `mcp__*` tools, provider-specific) can be routed by its
 * real name or result shape.
 *
 * A renderer is the highest-priority entry whose matcher accepts the node;
 * ties break by registration order. Convention: built-in (coarse-kind) renderers
 * register at priority 0, name-matched rich renderers above (10), shape-matched
 * between (5), and a catch-all below (-Infinity).
 *
 * See docs/specs/SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md.
 */

import type { JSX } from "solid-js";
import type { ToolNode } from "../../types";

export type ToolRenderer = (node: ToolNode) => JSX.Element;
export type ToolMatcher = (node: ToolNode) => boolean;

export interface ToolRendererEntry {
    priority: number;
    match: ToolMatcher;
    render: ToolRenderer;
    /** Stable label — used for de-dup on re-register and for tests/debugging. */
    label: string;
}

const entries: ToolRendererEntry[] = [];

/**
 * Register (or replace, by `label`) a renderer. Replacing on duplicate label
 * keeps HMR / double-import from stacking duplicate entries.
 */
export function registerToolRenderer(entry: ToolRendererEntry): void {
    const existing = entries.findIndex((e) => e.label === entry.label);
    if (existing >= 0) entries.splice(existing, 1);
    entries.push(entry);
}

/**
 * Pure resolution against an explicit entry list (for tests). Highest priority
 * wins; on a tie the earliest-registered entry wins.
 */
export function resolveFrom(
    list: readonly ToolRendererEntry[],
    node: ToolNode,
): ToolRenderer | null {
    let best: ToolRendererEntry | null = null;
    for (const e of list) {
        if (!e.match(node)) continue;
        if (best === null || e.priority > best.priority) best = e;
    }
    return best ? best.render : null;
}

/**
 * Resolve against the global registry. Returns null only when nothing matches;
 * callers keep a hard fallback for safety (a catch-all is normally registered).
 */
export function resolveToolRenderer(node: ToolNode): ToolRenderer | null {
    return resolveFrom(entries, node);
}

/** Test seam — the labels currently registered, in registration order. */
export function _registeredLabels(): string[] {
    return entries.map((e) => e.label);
}

// ── matcher helpers ──────────────────────────────────────────────────────────

/** The tool's real provider name when carried, else the coarse kind. */
export function toolNameOf(n: ToolNode): string {
    return n.toolName ?? n.tool;
}

/** Match the coarse `ToolNode.tool` kind. */
export const byKind =
    (...kinds: ToolNode["tool"][]): ToolMatcher =>
    (n) =>
        kinds.includes(n.tool);

/** Match the real provider tool name (falls back to the coarse kind). */
export const byName =
    (...names: string[]): ToolMatcher =>
    (n) =>
        names.includes(toolNameOf(n));

/** Match a tool-name prefix — e.g. `byNamePrefix("mcp__")`. */
export const byNamePrefix =
    (prefix: string): ToolMatcher =>
    (n) =>
        toolNameOf(n).startsWith(prefix);

/** Match on the result shape — e.g. `byShape(looksLikeSearchResults)`. */
export const byShape =
    (pred: (result: unknown) => boolean): ToolMatcher =>
    (n) =>
        pred(n.result);

/** Always matches — for a lowest-priority catch-all. */
export const anyTool: ToolMatcher = () => true;
