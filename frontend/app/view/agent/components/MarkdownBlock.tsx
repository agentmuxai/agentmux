// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MarkdownBlock - Renders markdown content from agent output
 */

import { Show, type JSX } from "solid-js";
import { useAgentMedia } from "../agent-media";
import { rowDisclosure } from "../virtualization/disclosure";
import { createMarkdownPeek, createStreamedView, type MarkdownBlockProps } from "./MarkdownBlockState";
import { FoldedMarkdown, PeekedMarkdown } from "./MarkdownBlockParts";

export const MarkdownBlock = (props: MarkdownBlockProps): JSX.Element => {
    // Don't destructure `node` — the streaming buffer keeps this row
    // mounted across token deltas, and useAgentStream replaces the
    // node reference for each chunk. A destructured `node` would
    // capture the first reference and freeze. Access props.node.X at
    // each site so Solid's reactivity tracks the read. (codex P1 on
    // PR #786 / virt redesign.)

    // Canceled thinking — orphan-scrub flipped this on at the last
    // SessionEnd or HistoryRestored. Render collapsed by default
    // with a "⏹ Canceled" label; click to expand the partial
    // content. Spec:
    // `docs/specs/SPEC_ORPHAN_THINKING_NODES_2026_05_27.md`.
    const isCanceled = (): boolean => props.node.metadata?.canceled === true;
    // A thought the pane's view folds or opens (Thinking is Collapsed or Expanded, virtualization/verbosity.ts):
    // it gets the same one-line header a canceled thought has. In Full it has none and stays open.
    const isFoldable = (): boolean => props.node.metadata?.thinking === true && props.node.viewMode != null;
    // Open state lives in documentState (pinnedNodes) like every other row,
    // not a local signal, so the layout slice sees it
    // (SPEC_AGENT_PANE_ROW_DISCLOSURE_2026_09_26 §2.5).
    const expanded = (): boolean => rowDisclosure(props.node, { pinned: props.pinned }).open;

    const view = createStreamedView(props);
    // The agent's own text: local images may show inline (agent-media.tsx).
    const media = useAgentMedia();
    const peek = createMarkdownPeek(props);

    return (
        <Show
            when={isCanceled() || isFoldable()}
            fallback={<PeekedMarkdown b={props} view={view} media={media} peek={peek} />}
        >
            <FoldedMarkdown b={props} view={view} media={media} isCanceled={isCanceled} expanded={expanded} />
        </Show>
    );
};

MarkdownBlock.displayName = "MarkdownBlock";
