// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolReferences — a ToolSearch result as the tools it loaded, one chip each.
 *
 * Claude Code's ToolSearch returns `[{type:"tool_reference", tool_name}]`
 * blocks, which the translator passes through (they aren't text), and which
 * the shape-matched RecordTable used to render as a `type | tool_name` table.
 * SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.7 (F6).
 */

import { For, Show, type JSX } from "solid-js";
import type { ToolNode } from "../../types";
import { CompactResult } from "../CompactResult";
import { mcpDisplayName } from "../tool-header";
import { byName, registerToolRenderer } from "./registry";

/** The referenced tool names, or null unless the result is a non-empty array
 *  made only of tool_reference blocks. */
export function toolReferenceNames(result: unknown): string[] | null {
    if (!Array.isArray(result) || result.length === 0) return null;
    const names: string[] = [];
    for (const b of result) {
        const o = (b ?? {}) as { type?: unknown; tool_name?: unknown };
        if (o.type !== "tool_reference" || typeof o.tool_name !== "string") return null;
        names.push(o.tool_name);
    }
    return names;
}

export function ToolReferences(props: { node: ToolNode }): JSX.Element {
    const names = () => toolReferenceNames(props.node.result);
    return (
        <Show
            when={names()}
            fallback={<CompactResult tool={props.node.tool} params={props.node.params as any} result={props.node.result} />}
        >
            <div class="agent-tool-references">
                <For each={names()!}>
                    {(name) => (
                        <span class="agent-tool-reference" title={name}>
                            {mcpDisplayName(name) ?? name}
                        </span>
                    )}
                </For>
            </div>
        </Show>
    );
}

ToolReferences.displayName = "ToolReferences";

registerToolRenderer({
    priority: 10,
    label: "tool:search-references",
    match: byName("ToolSearch"),
    render: (node) => <ToolReferences node={node} />,
});
