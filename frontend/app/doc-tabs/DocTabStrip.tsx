// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The document-tab strip: a pane's documents, drawn with the shared
 * `PaneTabStrip`, under the pane header. The Editor and Media both draw
 * theirs with it, each from its `DocTabsModel`.
 * docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §4.1,
 * docs/specs/PLAN_EDITOR_ON_SHARED_DOC_TABS_2026_10_10.md.
 *
 * Tabs drag as document tabs (`PaneTabStrip`'s `docDrag`): their own drag
 * kind, which never tears off into a window. Dropped on another tab of the
 * strip, a tab moves there; reorder by keys still works
 * (Ctrl+Shift+PageUp/PageDown). A tab of another pane of the same type
 * dropped on one of these moves into this pane, beside it (doc-tab-hosts.ts).
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md.
 */

import { registerContextMenuRegion } from "@/app/block/context-menu-region";
import { PaneTabStrip } from "@/app/element/PaneTabStrip";
import { onCleanup, onMount, Show, type JSX } from "solid-js";
import type { DocTab } from "./doc-tabs";
import type { DocTabsModel } from "./doc-tabs-controller";
import { dropDocTab } from "./doc-tab-hosts";
import { moveDocTabToNewPane } from "./doc-tab-new-pane";
import "./doc-tabs.scss";
import { keyLabel } from "@/app/keybindings";

export function DocTabStrip<P>(props: {
    ctl: DocTabsModel<P>;
    /** Tooltip per tab (a full path); the title when omitted. */
    tooltipOf?: (tab: DocTab<P>) => string;
    addTitle?: string;
    /** The pane type ("media"): tabs move only between panes of one type. */
    docType: string;
    /** The block whose tabs these are. */
    blockId: string;
    /** Whether a tab can be dragged. */
    canDrag?: (tab: DocTab<P>) => boolean;
    /** Before the tabs, in the strip's row (the Editor's file tree toggle). */
    leading?: JSX.Element;
    /** Whether the row shows; `ctl.showStrip()` when omitted. */
    show?: () => boolean;
    /** A tab's label in place of its title (the Editor's Save As box), or
     *  undefined for the title. */
    renderLabel?: (tab: DocTab<P>) => JSX.Element | undefined;
    /** Classes a pane adds to a tab. */
    tabClass?: (tab: DocTab<P>) => Record<string, boolean>;
}): JSX.Element {
    const ctl = props.ctl;
    // A right-click on a tab adds its items to the pane menu (blockframe's
    // body menu reads the region). The tab is noted as the click starts, so
    // the items, built a moment later, act on it.
    let rightClicked: string | null = null;
    const noteTab = (e: MouseEvent) => {
        rightClicked = (e.target as Element | null)?.closest?.("[data-tab-id]")?.getAttribute("data-tab-id") ?? null;
    };
    const tabMenu = (): ContextMenuItem[] => {
        const id = rightClicked;
        if (!id || !ctl.tabs().some((t) => t.id === id)) return [];
        return [
            { label: "Move to New Pane", click: () => void moveDocTabToNewPane(props.blockId, id, props.blockId, "right") },
            { label: "Close Tab", click: () => void ctl.close(id) },
        ];
    };
    const bindMenu = (el: HTMLDivElement) => {
        onMount(() => {
            el.addEventListener("contextmenu", noteTab, true);
            const off = registerContextMenuRegion(el, { items: tabMenu });
            onCleanup(() => {
                el.removeEventListener("contextmenu", noteTab, true);
                off();
            });
        });
    };
    return (
        <Show when={props.show?.() ?? ctl.showStrip()}>
            <div class="doc-tab-strip" role="presentation" ref={bindMenu}>
                {props.leading}
                <Show when={ctl.tabs().length > 0}>
                    <PaneTabStrip<DocTab<P>>
                        tabs={ctl.tabs()}
                        activeId={ctl.activeId()}
                        animateWidth
                        getId={(t) => t.id}
                        getLabel={(t) => t.title}
                        getIcon={(t) => (t.icon ? <i class={`fa fa-${t.icon}`} aria-hidden="true" /> : <></>)}
                        getTooltip={(t) => {
                            const base = props.tooltipOf?.(t) ?? t.title;
                            return t.preview ? `${base} (preview, double-click to keep)` : base;
                        }}
                        getAttention={(t) => !!t.dirty}
                        getTabClass={(t) => ({
                            "doc-tab--preview": t.preview,
                            "doc-tab--pinned": t.pinned,
                            "doc-tab--no-icon": !t.icon,
                            ...(props.tabClass?.(t) ?? {}),
                        })}
                        onActivate={(id) => ctl.activate(id)}
                        onClose={(id) => void ctl.close(id)}
                        onTabDoubleClick={(t) => {
                            if (t.preview) ctl.promote(t.id);
                        }}
                        renderLabel={
                            props.renderLabel
                                ? (t) => props.renderLabel!(t) ?? <span class="pane-tab-label">{t.title}</span>
                                : undefined
                        }
                        onAdd={ctl.canAddNew() ? () => void ctl.newDocument() : undefined}
                        addTitle={props.addTitle ?? `New tab (${keyLabel("ctrl+t")})`}
                        docDrag={{
                            docType: props.docType,
                            blockId: props.blockId,
                            canDrag: (id) => {
                                const tab = ctl.tabs().find((t) => t.id === id);
                                return !!tab && (props.canDrag?.(tab) ?? true);
                            },
                            onReorder: (id, targetId, position) => ctl.moveTo(id, targetId, position),
                            onReceive: (sourceBlockId, tabId, at) => dropDocTab(sourceBlockId, tabId, props.blockId, at),
                        }}
                    />
                </Show>
            </div>
        </Show>
    );
}
