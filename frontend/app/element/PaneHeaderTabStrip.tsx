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
 * reimplementation of any part of it: the tab strip goes in through
 * `BlockFrame_Header`'s `leadingTabStrip` prop (`blocktypes.ts`), which
 * renders in place of the iconview (icon+title+blockid). Every other row
 * element (ConnectionButton, header text elems, error boundary, drag handle,
 * context menu, EndIcons) is untouched.
 *
 * The strip is ALWAYS passed, lone tab included: a Pane header always shows
 * its tabs. There used to be a `pane:tabstrip = "multi-only"` setting that
 * swapped a lone tab back to the plain iconview; it was removed because a
 * pane without a pill silently lost every per-tab feature (drag, drop,
 * activity flash). SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §8.
 *
 * Spec: docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md §4.1.
 * Plan: docs/specs/PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md
 * Task A1 (component), Task B2/B3 (agent/term migration onto it).
 */

import type { Accessor, JSX } from "solid-js";
import { BlockFrame_Header } from "@/app/block/blockframe";
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
            // A Pane's own header: ids are blockIds, so a pill can be the
            // target of an activity flash (SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md).
            flashOnActivity
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
            // The window tab this pane lives in — enables dragging a pill out
            // of the window into a floating pane (PaneTabStrip's own doc).
            sourceTabId={props.nodeModel.layoutModel?.tabAtom?.()?.oid}
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
            leadingTabStrip={pillStrip}
            headerBgOverride={props.headerBgOverride}
        />
    );
}
