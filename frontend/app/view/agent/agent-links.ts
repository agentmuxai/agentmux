// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the agent pane tells the links layer about an agent's message (spec §7, row 2). */

import type { SurfaceContext } from "@/app/links/link-context";
import type { MarkdownMediaOpts } from "@/app/element/markdown-media";

/** The pane's author for its messages; none for text that is not the agent's own (a tool result), which is not linked. */
export function paneSurface(media: MarkdownMediaOpts | undefined): SurfaceContext | undefined {
    if (!media?.blockId) return undefined;
    return { author: { kind: "agent", id: media.blockId, confined: Boolean(media.confined) } };
}
