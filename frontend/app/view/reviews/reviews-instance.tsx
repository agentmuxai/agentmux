// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Reviews pane's instance, loaded with the first one (reviews.tsx). It follows srv's list for as long as the pane lives.

import type { PaneTabHostContext, PaneTabInstance } from "@/app/store/pane-tab-registry";
import { useReviews } from "./reviews-store";
import { ReviewsView } from "./reviews-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const data = useReviews();
    return {
        component: () => <ReviewsView data={data} ctx={ctx} />,
        liveTitle: () => ({ text: "Reviews" }),
    };
}
