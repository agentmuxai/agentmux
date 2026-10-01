// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Reading an unloaded tool result back from the transcript when its row is
 * opened: one line, at the position the node recorded, parsed with the same
 * parser as replay. docs/specs/SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_2026_10_01.md §3.4.
 *
 * The pane provides the loader through context (like `agent-media.tsx`), so
 * the virtualized render path doesn't grow a prop it only forwards.
 */

import { createContext, useContext, type JSX } from "solid-js";
import { parseHistoryLines } from "./parseHistoryLines";
import type { ToolNode, ToolResult } from "./types";

/** Reads one transcript line of a block's output: `null` lines on failure. */
export type ReadLine = (offset: number) => Promise<{ lines?: string[] | null; stream?: string; gen?: string }>;

/**
 * The node's result, read back from its recorded line, or null when it's
 * gone: the stream or generation moved on (replaced, truncated), the line is
 * missing, or the line no longer holds this tool's result.
 */
export async function readToolResult(node: ToolNode, outputFormat: string, readLine: ReadLine): Promise<ToolResult | null> {
    const src = node.resultSource;
    if (!src) return null;
    let resp: Awaited<ReturnType<ReadLine>>;
    try {
        resp = await readLine(src.line);
    } catch {
        return null;
    }
    if (resp.stream !== src.stream || resp.gen !== src.gen) return null;
    const line = resp.lines?.[0];
    if (!line) return null;
    const { nodes } = parseHistoryLines([line], outputFormat);
    const found = nodes.find((n) => n.type === "tool" && n.id === node.id) as ToolNode | undefined;
    return found?.result ?? null;
}

/** Loads a node's unloaded result into the pane; resolves false when it can't. */
export type ToolResultLoader = (node: ToolNode) => Promise<boolean>;

const ToolResultLoaderContext = createContext<ToolResultLoader | undefined>(undefined);

export function ToolResultLoaderProvider(props: { load: ToolResultLoader; children: JSX.Element }): JSX.Element {
    return <ToolResultLoaderContext.Provider value={props.load}>{props.children}</ToolResultLoaderContext.Provider>;
}

/** The pane's loader, or undefined outside an agent pane. */
export function useToolResultLoader(): ToolResultLoader | undefined {
    return useContext(ToolResultLoaderContext);
}
