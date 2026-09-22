// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneHeaderTabStrip — the unified Pane header: ONE row containing the tab
 * strip and the pane-level chrome (ConnectionButton, status text, minimize/
 * magnify/close). The chrome this replaced (the former
 * `AgentPaneChrome`/`TermPaneChrome`) rendered those as TWO separate rows
 * — a full `BlockFrame_Header` on top, `PaneTabStrip` below it.
 *
 * Deliberately a THIN wrapper around the real `BlockFrame_Header`, not a
 * reimplementation of any part of it. `BlockFrame_Header` gained two props
 * (`blocktypes.ts`) for this:
 *   - `leadingTabStrip` — when set, renders in place of the iconview
 *     (icon+title+blockid). Set whenever the pane has tabs, which is always
 *     (PaneChrome includes a lone tab), unless `pane:tabstrip` is
 *     "multi-only" and there's just one.
 *   - `trailingAddButton` — the "+" beside the iconview in that
 *     "multi-only" lone-tab case.
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
    /** Explicit header-row background, overriding the active block's own
     *  pane color. Set by PaneChrome when this pane's tabs don't agree on
     *  a single color — see SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md.
     *  Passed straight through to BlockFrame_Header; this component adds
     *  no policy of its own. */
    headerBgOverride?: string;
}

export function PaneHeaderTabStrip<T>(props: PaneHeaderTabStripProps<T>): JSX.Element {
    // "always" (default): every tab is a pill, including a lone one, so the
    // header looks the same at one tab as at many. "multi-only" opts a lone
    // tab back into the plain icon+title header.
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
            animateWidth={props.animateWidth}
            getId={props.getId}
            getLabel={props.getLabel}
            getIcon={props.getIcon}
            getTooltip={props.getTooltip}
            getAttention={props.getAttention}
            getTabClass={props.getTabClass}
            getColor={props.getColor}
            onActivate={props.onActivate}
            onClose={props.onClose}
            onTabDoubleClick={props.onTabDoubleClick}
            renderLabel={props.renderLabel}
            onAdd={props.onAdd}
            addTitle={props.addTitle}
            addLabel={props.addLabel}
            // §3.6 — this is a Pane's own header row (the whole-pane drag
            // handle, blockframe.tsx's `data-role="block-header"`), the one
            // strip usage where reserving drag space after the "+" matters.
            reserveDragHandle
            onReorder={props.onReorder}
            onReceiveForeignTab={props.onReceiveForeignTab}
            // Derived from `nodeModel` (this strip's own Pane) rather than
            // requiring a redundant caller-supplied prop — `PaneTabStrip`'s
            // `canDrop` uses it to reject a pill dragged from a DIFFERENT
            // pane (see that prop's own doc comment). ReAgent P1 on PR #3444.
            paneKey={props.nodeModel.nodeId}
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
            headerBgOverride={props.headerBgOverride}
        />
    );
}
