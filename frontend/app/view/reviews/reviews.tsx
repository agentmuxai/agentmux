// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Reviews module barrel: its native pane tab manifest. Its code loads with the first Reviews pane (reviews-instance.tsx).
import { lazyPaneTab } from "@/app/block/lazy-pane-tab";

/** Reviews as a native pane tab: every task's pull request and its review state (docs/specs/SPEC_REVIEWS_PANE_2026_11_15.md). */
export const reviewsPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "reviews",
    label: "Reviews",
    title: () => "Reviews",
    icon: "code-pull-request",
    defaultColor: "blue",
    capabilities: { paneZoom: {}, noPadding: true },
    members: ["liveTitle"],
    load: () => import("./reviews-instance"),
});
