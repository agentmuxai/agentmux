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
import { Show } from "solid-js";
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

    /** Identity label shown whenever `tabs` is empty, REGARDLESS of
     *  whether `onAdd` is set — i.e. whenever there is no per-tab pill to
     *  carry the Pane's identity, something still must (ReAgent P1 on PR
     *  #3309: an earlier version of this component only showed
     *  `emptyLabel` when `onAdd` was ALSO unset, so agent's/term's single-
     *  conversation/single-shell state — `tabs=[]`, `onAdd` set — rendered
     *  a bare "+" with no title at all, the overwhelmingly common case).
     *  Rendered ALONGSIDE the tab strip (which still renders its own "+"
     *  when `onAdd` is set), never as an alternative to it — see
     *  `leadingContent` below. */
    emptyLabel?: string;
}

export function PaneHeaderTabStrip<T>(props: PaneHeaderTabStripProps<T>): JSX.Element {
    // §7 resolution 1: "always" is the default — a single-tab Pane still
    // shows a one-pill strip. "multi-only" is the opt-out: a single-tab
    // Pane shows a plain title instead of that one pill.
    //
    // For BOTH current callers (agent/term), `multi-only`'s own branch
    // below is unreachable in practice: `visibleTabs()`/`visibleTermTabs()`
    // already collapse a real single-tab state down to an EMPTY array
    // before it ever reaches this component (agent-view.tsx/term.tsx's own
    // "a lone tab shows no self-pill" convention, predating this redesign)
    // — so `props.tabs.length` here is only ever 0 or ≥2 for them, never
    // exactly 1. `multi-only` and `always` therefore render identically for
    // agent/term today; genuine, forward-looking infrastructure for widget
    // types that DON'T have that convention (most of §5's later rollout).
    // Not a bug to "fix" by changing agent/term's existing convention.
    const tabStripSetting = getSettingsKeyAtom("pane:tabstrip");
    const showMultiOnlyPlainTitle = () => (tabStripSetting() ?? "always") === "multi-only" && props.tabs.length === 1;

    // The identity label to show alongside the strip when there's no pill
    // to carry it — `tabs.length === 0` (regardless of `onAdd`; ReAgent P1
    // on PR #3309 — see `emptyLabel`'s own doc comment for the bug this
    // fixes) or the `multi-only`-suppressed single pill.
    const identityLabel = () => {
        if (props.tabs.length === 0 && props.emptyLabel) return props.emptyLabel;
        if (showMultiOnlyPlainTitle()) return props.getLabel(props.tabs[0]);
        return null;
    };
    // What PaneTabStrip itself renders — suppressed to an empty list (just
    // its own "+", if `onAdd` is set) whenever `identityLabel` is already
    // carrying this Pane's one-and-only tab's identity, so the same title
    // never appears twice.
    const stripTabs = () => (showMultiOnlyPlainTitle() ? [] : props.tabs);

    const leadingTabStrip = (
        <>
            <Show when={identityLabel()}>
                <div class="block-frame-view-type">{identityLabel()}</div>
            </Show>
            <PaneTabStrip
                tabs={stripTabs()}
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
        </>
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
