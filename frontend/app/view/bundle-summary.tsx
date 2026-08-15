// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// BundleSummaryPanel — the read-only summary surface that the
// agent-settings Identity / Memory tabs now render (PR 5 of
// docs/specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md, §5 decision 3).
//
// Before this PR the `view: "identity"` / `view: "memory"` panes
// rendered the full-CRUD `IdentityManagerBody` / `MemoryManagerBody`.
// §4 of the spec consolidated all bundle CRUD into the Armory pane;
// the per-agent settings tabs are now *consumers*, not editors.
//
// This panel is CRUD-free: it shows a short pointer explaining that
// bundles are app-wide data managed in one place, plus a "Manage in
// Identity & Memory" button that opens the Armory via
// `openOrFocusPaneByView("armory")`.
//
// Per-agent bundle resolution — DATA GAP, CLOSED (2026-08-15) for the one
// live remaining consumer. `view: "identity"` blocks stopped rendering
// this panel entirely back in Armory Phase 5
// (SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md §1.3
// — see `identity-pane-view.tsx`); `view: "memory"` (`memory-view.tsx`)
// was the one spot still stuck on the pointer-only form. Fixed the same
// way Phase 5 fixed Identity: an optional `agentId` prop, threaded from
// the block's own `meta.agentId` (`MemoryViewModel.agentId`, mirroring
// `IdentityPaneViewModel.agentId`). When present, this resolves the
// agent's OWN dedicated ABF bundle via `AgentDefinition.memory_id`
// (ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §3.1 — the
// definition-level, readonly-after-creation binding, not the
// per-instance-launch `AgentInstance.memory_id` the original DATA GAP
// note below was written against) and shows its name + provider inline,
// with a "Edit in Identity & Memory" link into Armory. `agentId` absent
// (or resolution still in flight / failed) degrades to the original
// context-free pointer-only form — unchanged for every OTHER caller of
// this component and for legacy blocks with no agent context at all.
//
// Both `view: "identity"`/`"memory"` block types are themselves legacy —
// no live UI flow creates them anymore (superseded by the agent pane's
// "Stash" modal, `AgentStashModal.tsx`); this only matters for pre-
// existing persisted layouts that still have one open.

import { createMemo, createResource, Show, type JSX } from "solid-js";

import { openOrFocusPaneByView } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { PROVIDERS } from "@/app/view/agent/providers/catalog";

import "./bundle-summary.scss";

interface BundleSummaryPanelProps {
    /** "Identity" | "Bundle" — drives the heading + copy. */
    kind: "Identity" | "Bundle";
    /** The agent this panel's block belongs to, if any (`meta.agentId`) —
     *  see the module doc comment above. `undefined` renders the original
     *  context-free pointer-only form. */
    agentId?: string;
}

export const BundleSummaryPanel = (props: BundleSummaryPanelProps): JSX.Element => {
    const agents = useAgentDefinitions();
    const boundBundleId = createMemo(() => {
        const id = props.agentId;
        if (!id) return undefined;
        const memoryId = agents().find((a) => a.id === id)?.memory_id;
        return memoryId || undefined;
    });
    // createResource re-fetches whenever boundBundleId() changes (agent
    // switches, or the list finishes loading and a previously-undefined
    // id resolves to a real one).
    const [boundBundle] = createResource(boundBundleId, (id) =>
        RpcApi.GetMemoryCommand(TabRpcClient, { id }).catch(() => undefined),
    );
    // Identity items are still called "identity bundles"; the config
    // collections are now branded "Armory Bundle Format (ABF)". `title` is
    // the heading (room for the full name); `sentenceLabel` reads naturally
    // in running prose below.
    const title = props.kind === "Identity" ? "Identity bundles" : "Armory Bundle Format (ABF)";
    const sentenceLabel = props.kind === "Identity" ? "Identity bundles" : "Bundles";
    const lowerPlural = props.kind === "Identity" ? "identities" : "bundles";

    return (
        <div class="bundle-summary">
            <div class="bundle-summary-inner">
                <h2 class="bundle-summary-title">{title}</h2>

                <Show when={props.agentId && boundBundle()}>
                    {(bundle) => (
                        <div class="bundle-summary-bound">
                            <p class="bundle-summary-bound-label">This agent's own ABF</p>
                            <p class="bundle-summary-bound-name">{bundle().name}</p>
                            <Show when={bundle().provider}>
                                <p class="bundle-summary-bound-provider">
                                    {PROVIDERS[bundle().provider ?? ""]?.displayName ?? bundle().provider}
                                </p>
                            </Show>
                        </div>
                    )}
                </Show>
                <Show when={props.agentId && boundBundleId() && !boundBundle.loading && !boundBundle()}>
                    <p class="bundle-summary-body bundle-summary-hint">
                        This agent's ABF bundle couldn't be loaded (it may have been deleted).
                    </p>
                </Show>
                <Show when={props.agentId && agents().length > 0 && !boundBundleId()}>
                    <p class="bundle-summary-body bundle-summary-hint">
                        This agent has no ABF bundle of its own yet.
                    </p>
                </Show>

                <p class="bundle-summary-body">
                    {sentenceLabel} are app-wide data, shared across every agent and
                    window. They are now created, edited, and deleted in one
                    place — the <strong>Identity &amp; Memory</strong>{" "}
                    manager, opened from the hamburger menu.
                </p>
                <p class="bundle-summary-body bundle-summary-hint">
                    This settings tab no longer manages {lowerPlural}; open the
                    manager to make changes.
                </p>
                <button
                    type="button"
                    class="bundle-summary-btn"
                    onClick={() => void openOrFocusPaneByView("armory")}
                >
                    {props.agentId && boundBundle() ? "Edit in Identity & Memory" : "Manage in Identity & Memory"}
                </button>
            </div>
        </div>
    );
};

BundleSummaryPanel.displayName = "BundleSummaryPanel";
