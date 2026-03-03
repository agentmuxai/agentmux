// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * MarkdownBlock - Renders markdown content from agent output
 */

import { Markdown } from "@/app/element/markdown";
import clsx from "clsx";
import React, { memo } from "react";
import type { MarkdownNode } from "../types";

interface MarkdownBlockProps {
    node: MarkdownNode;
}

export const MarkdownBlock: React.FC<MarkdownBlockProps> = memo(({ node }) => {
    return (
        <div
            className={clsx("agent-markdown-block", {
                "thinking-block": node.metadata?.thinking,
            })}
        >
            <Markdown text={node.content} />
        </div>
    );
});

MarkdownBlock.displayName = "MarkdownBlock";
