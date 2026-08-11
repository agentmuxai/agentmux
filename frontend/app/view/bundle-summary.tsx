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
// This panel is intentionally context-free and CRUD-free: it shows a
// short pointer explaining that bundles are app-wide data managed in one
// place, plus a "Manage in Identity & Memory" button that opens the
// Armory via `openOrFocusPaneByView("armory")`.
//
// Per-agent bundle resolution — DATA GAP: the spec's ideal is to show
// "this agent uses Identity: X". The launched identity/memory bundle ids
// live on the `AgentInstance` DB row (`identity_id` / `memory_id`),
// reachable only from the *agent pane's* block meta (`agentInstanceId`).
// The `view: "identity"` / `view: "memory"` panes are SEPARATE blocks
// with no link back to the launching agent, so the per-agent bundle is
// not resolvable from this surface. The panel therefore degrades to the
// pointer-only form. See the PR report for detail.

import { type JSX } from "solid-js";

import { openOrFocusPaneByView } from "@/app/store/global";

import "./bundle-summary.scss";

interface BundleSummaryPanelProps {
    /** "Identity" | "Bundle" — drives the heading + copy. */
    kind: "Identity" | "Bundle";
}

export const BundleSummaryPanel = (props: BundleSummaryPanelProps): JSX.Element => {
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
                    Manage in Identity &amp; Memory
                </button>
            </div>
        </div>
    );
};

BundleSummaryPanel.displayName = "BundleSummaryPanel";
