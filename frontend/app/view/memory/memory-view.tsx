// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory pane view — the agent-settings `view: "memory"` tab.
//
// PR 5 of specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md (§5 decision 3) DEMOTED
// this tab from full CRUD to a read-only summary. Full Memory-bundle
// management now lives in exactly one place: the hamburger "Identity &
// Memory" manager (`BundleManagerModal`).
//
// This file no longer renders `MemoryManagerBody` — it renders the
// CRUD-free `<BundleSummaryPanel/>`, which points the user at the
// app-wide manager. `agentId` (`MemoryViewModel.agentId`, mirroring
// `IdentityPaneViewModel.agentId`) closes bundle-summary.tsx's own
// documented DATA GAP: when this block was opened with `meta.agentId`
// set, the panel resolves and shows that specific agent's own bound ABF
// bundle instead of staying purely generic. The `MemoryViewModel` is
// still the registered `viewComponent` ViewModel (BlockRegistry needs
// one), and the context-free `MemoryManager` (used by the hamburger
// modal) is untouched — only this agent-settings wrapper changed.

import { type JSX } from "solid-js";

import { BundleSummaryPanel } from "@/app/view/bundle-summary";
import type { MemoryViewModel } from "./memory-model";

interface MemoryViewProps {
    model: MemoryViewModel;
}

export const MemoryView = (props: MemoryViewProps): JSX.Element => {
    return <BundleSummaryPanel kind="Bundle" agentId={props.model.agentId()} />;
};
