// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MarkdownBlock - Renders markdown content from agent output
 */

import { Markdown } from "@/app/element/markdown";
import clsx from "clsx";
import { type JSX } from "solid-js";
import type { MarkdownNode } from "../types";

interface MarkdownBlockProps {
    node: MarkdownNode;
}

export const MarkdownBlock = (props: MarkdownBlockProps): JSX.Element => {
    // Don't destructure `node` — the streaming buffer keeps this row
    // mounted across token deltas, and useAgentStream replaces the
    // node reference for each chunk. A destructured `node` would
    // capture the first reference and freeze. Access props.node.X at
    // each site so Solid's reactivity tracks the read. (codex P1 on
    // PR #786 / virt redesign.)
    return (
        <div
            class={clsx("agent-markdown-block", {
                "thinking-block": props.node.metadata?.thinking,
            })}
        >
            <Markdown text={props.node.content} />
        </div>
    );
};

MarkdownBlock.displayName = "MarkdownBlock";
