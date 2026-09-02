// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Identity pane view — the agent-settings `view: "identity"` tab.
//
// PR 5 of docs/specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md (§5 decision 3) DEMOTED
// this tab from full CRUD to a read-only, context-free `<BundleSummaryPanel/>`
// stub (it couldn't resolve which agent it belonged to — see that file's
// former DATA GAP comment). Armory Phase 5
// (docs/specs/SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md
// §1.3) closed that gap: this pane now reads its own block's `agentId`
// (`IdentityPaneViewModel.agentId`, plumbed from `meta.agentId` — the same
// field the agent pane's own block already uses) and renders that
// specific agent's linked accounts via `<AgentIdentityLinksPanel/>`. This
// also took over the job the Armory pane's now-removed "Identities" tab
// (`AgentIdentitiesPanel`) used to do.

import { type JSX } from "solid-js";

import { AgentIdentityLinksPanel } from "./agent-identity-links-panel";
import type { IdentityPaneViewModel } from "./identity-pane-model";

interface IdentityPaneViewProps {
    model: IdentityPaneViewModel;
}

export const IdentityPaneView = (props: IdentityPaneViewProps): JSX.Element => {
    return <AgentIdentityLinksPanel agentId={props.model.agentId()} />;
};
