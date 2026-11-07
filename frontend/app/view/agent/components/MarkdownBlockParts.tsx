// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The markdown row's two shapes: the text with its peek tooltip, and a thought folded under a header. */

import clsx from "clsx";
import { Markdown } from "@/app/element/markdown";
import { Button } from "@/app/element/ui/Button";
import { Show, type Accessor, type JSX } from "solid-js";
import type { useAgentMedia } from "../agent-media";
import { PeekOverlay } from "./PeekOverlay";
import { PeekMetaRow } from "./PeekMetaRow";
import type { MarkdownBlockProps, MarkdownPeek, StreamedView } from "./MarkdownBlockState";

type Media = ReturnType<typeof useAgentMedia>;

/** The text, with the peek overlay anchored to it. */
export function PeekedMarkdown(props: {
    b: MarkdownBlockProps;
    view: Accessor<StreamedView>;
    media: Media;
    peek: MarkdownPeek;
}): JSX.Element {
    const b = props.b;
    const view = props.view;
    const peek = props.peek;
    // Shared anchor + peek overlay for BOTH thinking clumps and
    // regular assistant text — see createMarkdownPeek for why this no
    // longer special-cases thinking.
    // Not gated on an `expanded()` check here — markdown blocks have
    // no pin/expand state to collide with (unlike ToolBlock).
    return (
        <div
            ref={peek.setPeekRowEl}
            class="agent-markdown-peek-anchor"
            onMouseEnter={peek.handlePeekEnter}
            onMouseLeave={peek.handlePeekLeave}
        >
            <div
                class={clsx("agent-markdown-block", {
                    "thinking-block": b.node.metadata?.thinking === true,
                })}
            >
                {/* scrollable={false}: per-block scroll is wrong inside the
                    virtualized document, which owns the scroll. (It also avoided
                    the replaceChild crash, #1326, from the OverlayScrollbars that
                    scrollable markdown used before it was removed.) */}
                <Markdown text={view().text} streaming={view().streaming} scrollable={false} media={props.media()} />
            </div>
            {/* Peek overlay — see ToolBlock.tsx's identical pattern
                and PeekOverlay.tsx. */}
            <PeekOverlay
                show={peek.isPeeking() && (peek.peekTimeText() != null || peek.peekEstimateText() != null)}
                rowEl={peek.peekRowEl}
            >
                <PeekMetaRow time={peek.peekTimeText()} tokens={peek.peekEstimateText()} />
            </PeekOverlay>
        </div>
    );
}

/** A canceled or view-folded thought: one header line, and the thought under it when opened. */
export function FoldedMarkdown(props: {
    b: MarkdownBlockProps;
    view: Accessor<StreamedView>;
    media: Media;
    isCanceled: () => boolean;
    expanded: () => boolean;
}): JSX.Element {
    const b = props.b;
    const view = props.view;
    const isCanceled = props.isCanceled;
    const expanded = props.expanded;
    return (
        <div class={clsx("agent-markdown-block markdown-canceled", { "thinking-block": !isCanceled() })}>
            <Button
                variant="link"
                class="markdown-canceled-header"
                onClick={() => b.onTogglePin?.()}
                aria-expanded={expanded()}
            >
                <span class="markdown-canceled-icon" aria-hidden="true">{isCanceled() ? "⏹" : ""}</span>
                <span class="markdown-canceled-label">
                    {isCanceled() ? "Canceled — partial thought" : "Thinking"}
                </span>
                <span class="markdown-canceled-chevron" aria-hidden="true">
                    {expanded() ? "▾" : "▸"}
                </span>
            </Button>
            <Show when={expanded()}>
                <div class="markdown-canceled-body">
                    <Markdown
                        text={isCanceled() ? b.node.content : view().text}
                        streaming={isCanceled() ? undefined : view().streaming}
                        scrollable={false}
                        media={props.media()}
                    />
                </div>
            </Show>
        </div>
    );
}
