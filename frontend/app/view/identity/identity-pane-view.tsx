// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Identity pane view — the agent-settings `view: "identity"` tab.
//
// PR 5 of specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md (§5 decision 3) DEMOTED
// this tab from full CRUD to a read-only summary.
//
// This file renders the context-free, CRUD-free `<BundleSummaryPanel/>`.
// The `IdentityPaneViewModel` is still the registered `viewComponent`
// ViewModel (BlockRegistry needs one). The full-CRUD `IdentityManager`
// this tab used to render (`identity-manager.tsx`) was deleted as dead
// code (issue #1624 PR-C follow-up) — its other intended mount point (a
// hamburger "Identity & Memory manager") was never actually built; the
// Armory pane's read-only "Identities" tab shipped instead.

import { type JSX } from "solid-js";

import { BundleSummaryPanel } from "@/app/view/bundle-summary";
import type { IdentityPaneViewModel } from "./identity-pane-model";

interface IdentityPaneViewProps {
    model: IdentityPaneViewModel;
}

export const IdentityPaneView = (_props: IdentityPaneViewProps): JSX.Element => {
    return <BundleSummaryPanel kind="Identity" />;
};
