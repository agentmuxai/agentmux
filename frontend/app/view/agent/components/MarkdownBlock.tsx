// Copyright 2024, Command Line Inc.
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

export const MarkdownBlock = ({ node }: MarkdownBlockProps): JSX.Element => {
    return (
        <div
            class={clsx("agent-markdown-block", {
                "thinking-block": node.metadata?.thinking,
            })}
        >
            <Markdown text={node.content} />
        </div>
    );
};

MarkdownBlock.displayName = "MarkdownBlock";
