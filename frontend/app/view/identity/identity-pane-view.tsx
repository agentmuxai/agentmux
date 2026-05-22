// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Identity pane view — the agent-settings `view: "identity"` tab.
//
// PR 5 of SPEC_BUNDLE_MANAGEMENT_2026_05_22.md (§5 decision 3) DEMOTED
// this tab from full CRUD to a read-only summary. Full Identity-bundle
// management now lives in exactly one place: the hamburger "Identity &
// Memory" manager (`BundleManagerModal`).
//
// This file no longer renders `IdentityManagerBody` — it renders the
// context-free, CRUD-free `<BundleSummaryPanel/>`, which points the user
// at the app-wide manager. The `IdentityPaneViewModel` is still the
// registered `viewComponent` ViewModel (BlockRegistry needs one), and
// the context-free `IdentityManager` (used by the hamburger modal) is
// untouched — only this agent-settings wrapper changed.

import { type JSX } from "solid-js";

import { BundleSummaryPanel } from "@/app/view/bundle-summary";
import type { IdentityPaneViewModel } from "./identity-pane-model";

interface IdentityPaneViewProps {
    model: IdentityPaneViewModel;
}

export const IdentityPaneView = (_props: IdentityPaneViewProps): JSX.Element => {
    return <BundleSummaryPanel kind="Identity" />;
};
