// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneHeaderTabStrip — the unified Pane header: ONE row containing the tab
 * strip and the pane-level chrome (ConnectionButton, status text, minimize/
 * magnify/close), where today's chrome (`AgentPaneChrome`/`TermPaneChrome`)
 * renders those as TWO separate rows — a full `BlockFrame_Header` on top,
 * `PaneTabStrip` below it.
 *
 * Deliberately a THIN wrapper around the real `BlockFrame_Header`, not a
 * reimplementation of any part of it. `BlockFrame_Header` gained two props
 * (`blocktypes.ts`) for this:
 *   - `leadingTabStrip` — when set, renders in place of the iconview
 *     (icon+title+blockid). Only set here when there are 2+ real Pane Tabs
 *     to show as pills — see `usePillStrip` below for why 0 or 1 does NOT
 *     take this branch (ReAgent P1 on PR #3309, round 2: an earlier version
 *     unconditionally overrode the iconview with a synthetic literal even
 *     for a lone conversation/shell, silently losing the real per-pane
 *     name, branded icon, and click-to-rename affordance the iconview's
 *     `ViewNameEditor` already provides correctly).
 *   - `trailingAddButton` — rendered right after the iconview ONLY when
 *     `leadingTabStrip` is unset, so a lone conversation/shell still gets
 *     its "add tab" affordance without losing its real identity.
 * Every other row element (ConnectionButton, header text elems, error
 * boundary, drag handle, context menu, EndIcons) is untouched either way.
 *
 * Spec: docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md §4.1.
 * Plan: docs/specs/PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md
 * Task A1 (component), Task B2/B3 (agent/term migration onto it).
 */

import type { Accessor, JSX } from "solid-js";
import { BlockFrame_Header } from "@/app/block/blockframe";
import { getSettingsKeyAtom } from "@/app/store/global";
import { NodeModel } from "@/layout/index";
import { PaneTabStrip, type PaneTabStripProps } from "./PaneTabStrip";

export interface PaneHeaderTabStripProps<T> extends PaneTabStripProps<T> {
    nodeModel: NodeModel;
    /** The ACTIVE stack member's ViewModel, reactive — re-passed by the
     *  caller on every switch. `null` renders BlockFrame_Header's own
     *  no-viewModel fallback path (mirrors the old headerElemNoView). */
    viewModel: ViewModel | null;
    /** The active member's blockId, reactive. Distinct from
     *  `nodeModel.blockId` (frozen for the leaf's whole lifetime) — see
     *  `NodeModel.blockId`'s own doc comment. */
    activeBlockId: Accessor<string>;
    /** Only needed by callers whose ViewModel sets `manageConnection`
     *  (today: term) — BlockFrame_Header's ConnectionButton slot. Omit for
     *  a view type that never manages a connection (today: agent). */
    connBtnRef?: { current: HTMLDivElement | null };
    changeConnModalAtom?: import("@/util/util").SignalAtom<boolean>;
    error?: Error;
}

export function PaneHeaderTabStrip<T>(props: PaneHeaderTabStripProps<T>): JSX.Element {
    // §7 resolution 1: "always" is the default — a single REAL pill still
    // takes the pill-strip branch below. "multi-only" is the opt-out: a
    // single tab instead falls through to the real-iconview branch (same
    // one 0 tabs already uses), showing the ViewModel's own name/icon
    // rather than a pill.
    //
    // For BOTH current callers (agent/term), the `=== 1` half of this is
    // unreachable in practice: `visibleTabs()`/`visibleTermTabs()` already
    // collapse a real single-tab state down to an EMPTY array (their own
    // "a lone tab shows no self-pill" convention, predating this redesign)
    // — so `props.tabs.length` here is only ever 0 or ≥2 for them. Genuine,
    // forward-looking infrastructure for widget types that DON'T have that
    // convention (most of §5's later rollout). Not a bug to "fix" by
    // changing agent/term's existing convention.
    const tabStripSetting = getSettingsKeyAtom("pane:tabstrip");
    const usePillStrip = () =>
        props.tabs.length >= 2 || (props.tabs.length === 1 && (tabStripSetting() ?? "always") !== "multi-only");

    // The "+"-only-or-nothing element for the real-iconview branch. Reuses
    // PaneTabStrip itself with an empty tabs array — already exactly the
    // "just the +" rendering agent/term relied on pre-redesign, so this is
    // not new behavior, just relocated.
    const addButtonOnly = (
        <PaneTabStrip
            tabs={[]}
            activeId={null}
            getId={props.getId}
            getLabel={props.getLabel}
            onActivate={props.onActivate}
            onAdd={props.onAdd}
            addTitle={props.addTitle}
            addLabel={props.addLabel}
        />
    );

    const pillStrip = (
        <PaneTabStrip
            tabs={props.tabs}
            activeId={props.activeId}
            zoomFactor={props.zoomFactor}
            animateWidth={props.animateWidth}
            getId={props.getId}
            getLabel={props.getLabel}
            getIcon={props.getIcon}
            getTooltip={props.getTooltip}
            getAttention={props.getAttention}
            getTabClass={props.getTabClass}
            onActivate={props.onActivate}
            onClose={props.onClose}
            onTabDoubleClick={props.onTabDoubleClick}
            renderLabel={props.renderLabel}
            onAdd={props.onAdd}
            addTitle={props.addTitle}
            addLabel={props.addLabel}
        />
    );

    return (
        <BlockFrame_Header
            nodeModel={props.nodeModel}
            viewModel={props.viewModel}
            preview={false}
            blockId={props.activeBlockId}
            connBtnRef={props.connBtnRef}
            changeConnModalAtom={props.changeConnModalAtom}
            error={props.error}
            leadingTabStrip={usePillStrip() ? pillStrip : undefined}
            trailingAddButton={addButtonOnly}
        />
    );
}
