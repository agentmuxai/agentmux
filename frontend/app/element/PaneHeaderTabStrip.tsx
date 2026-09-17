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
 * reimplementation of any part of it. `BlockFrame_Header` gained a
 * `leadingTabStrip` prop (`blocktypes.ts`) that, when set, renders in place
 * of its own `.block-frame-default-header-iconview` (icon+title+blockid)
 * while leaving every other row element — ConnectionButton, header text
 * elems, error-boundary affordance, drag handle, context menu, EndIcons —
 * completely untouched. Cherry-picking just `EndIcons` (an earlier version
 * of this file did that) turned out to silently drop real, live features
 * terminal panes depend on (the ConnectionButton, status text) — reusing
 * the whole header avoids that failure mode by construction: there is
 * nothing left to forget.
 *
 * Spec: docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md §4.1.
 * Plan: docs/specs/PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md
 * Task A1 (component), Task B2/B3 (agent/term migration onto it).
 */

import type { Accessor, JSX } from "solid-js";
import { Match, Switch } from "solid-js";
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

    /** Identity label shown when `tabs` is empty AND `onAdd` is also
     *  omitted — i.e. genuinely nothing to show (agent's fresh, unlaunched
     *  picker pane is the only current caller of this branch;
     *  tab-strip-visibility.ts's `shouldShowTabStrip`). When `tabs` is
     *  empty but `onAdd` IS set (agent's "launched, lone conversation —
     *  show just the '+'" state), `PaneTabStrip` already renders that
     *  correctly on its own; this prop is not consulted in that case. */
    emptyLabel?: string;
}

export function PaneHeaderTabStrip<T>(props: PaneHeaderTabStripProps<T>): JSX.Element {
    // §7 resolution 1: "always" is the default — a single-tab Pane still
    // shows a one-pill strip. "multi-only" is the opt-out: a single-tab
    // Pane shows a plain title instead, same row either way (never a
    // second row under either setting).
    const tabStripSetting = getSettingsKeyAtom("pane:tabstrip");
    const showMultiOnlyPlainTitle = () => (tabStripSetting() ?? "always") === "multi-only" && props.tabs.length === 1;
    const showEmptyLabel = () => props.tabs.length === 0 && !props.onAdd && !!props.emptyLabel;

    const leadingTabStrip = (
        <Switch
            fallback={
                <PaneTabStrip
                    tabs={props.tabs}
                    activeId={props.activeId}
                    zoomFactor={props.zoomFactor}
                    animateWidth={props.animateWidth}
                    getId={props.getId}
                    getLabel={props.getLabel}
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
            }
        >
            <Match when={showEmptyLabel()}>
                <div class="block-frame-view-type">{props.emptyLabel}</div>
            </Match>
            <Match when={showMultiOnlyPlainTitle()}>
                <div class="block-frame-view-type">{props.getLabel(props.tabs[0])}</div>
            </Match>
        </Switch>
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
            leadingTabStrip={leadingTabStrip}
        />
    );
}
