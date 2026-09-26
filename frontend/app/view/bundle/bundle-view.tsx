// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory pane view — the agent-settings `view: "memory"` tab.
//
// PR 5 of docs/specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md (§5 decision 3) DEMOTED
// this tab from full CRUD to a read-only summary. Full Memory-bundle
// management now lives in exactly one place: the hamburger "Identity &
// Memory" manager (`BundleManagerModal`).
//
// This file no longer renders `BundleManagerBody` — it renders the
// CRUD-free `<BundleSummaryPanel/>`, which points the user at the
// app-wide manager. `agentId` (the block's `meta.agentId`, like the
// identity pane's) closes bundle-summary.tsx's own documented DATA GAP:
// when this block was opened with `meta.agentId` set, the panel resolves
// and shows that specific agent's own bound ABF bundle instead of staying
// purely generic. A native pane tab (`memoryPaneTab`, bundle.tsx) with no
// model of its own; the context-free `BundleManager` (used by the
// hamburger modal) keeps `BundleViewModel`.

import { type Accessor, type JSX } from "solid-js";

import { BundleSummaryPanel } from "@/app/view/bundle-summary";

interface BundleViewProps {
    agentId: Accessor<string | undefined>;
}

export const BundleView = (props: BundleViewProps): JSX.Element => {
    return <BundleSummaryPanel kind="Bundle" agentId={props.agentId()} />;
};
