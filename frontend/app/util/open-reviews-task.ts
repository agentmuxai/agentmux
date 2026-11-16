// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";

/**
 * Open the Reviews pane (this tab's, or a new one) filtered to one task's PRs. `taskRef` is how the task is written:
 * `#12` on the default board, `work#12` on another. The view id and the meta key are the pane's own
 * (reviews-view.tsx's `REVIEWS_TASK_META`), which MetaType does not list.
 */
export function openReviewsForTask(taskRef: string): Promise<void> {
    const meta: MetaType & Record<string, unknown> = { "reviews:task": taskRef };
    return openOrFocusPaneByView("reviews", { meta: { view: "reviews", ...meta } }, meta);
}
