// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The document-tab strip: a pane's documents, drawn with the shared
 * `PaneTabStrip` (as the Editor's file tabs are), under the pane header.
 * docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §4.1.
 *
 * Tabs drag as document tabs (`PaneTabStrip`'s `docDrag`): their own drag
 * kind, which never tears off into a window. Dropped on another tab of the
 * strip, a tab moves there; reorder by keys still works
 * (Ctrl+Shift+PageUp/PageDown).
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md.
 */

import { PaneTabStrip } from "@/app/element/PaneTabStrip";
import { Show, type JSX } from "solid-js";
import type { DocTab } from "./doc-tabs";
import type { DocTabsController } from "./doc-tabs-controller";
import "./doc-tabs.scss";
import { keyLabel } from "@/app/keybindings";

export function DocTabStrip<P>(props: {
    ctl: DocTabsController<P>;
    /** Tooltip per tab (a full path); the title when omitted. */
    tooltipOf?: (tab: DocTab<P>) => string;
    addTitle?: string;
    /** The pane type ("media"): tabs move only between panes of one type. */
    docType: string;
    /** The block whose tabs these are. */
    blockId: string;
    /** Whether a tab can be dragged (an empty one has nothing to move). */
    canDrag?: (tab: DocTab<P>) => boolean;
}): JSX.Element {
    const ctl = props.ctl;
    return (
        <Show when={ctl.showStrip()}>
            <div class="doc-tab-strip" role="presentation">
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
                    getTabClass={(t) => ({ "doc-tab--preview": t.preview, "doc-tab--pinned": t.pinned })}
                    onActivate={(id) => ctl.activate(id)}
                    onClose={(id) => void ctl.close(id)}
                    onTabDoubleClick={(t) => {
                        if (t.preview) ctl.promote(t.id);
                    }}
                    onAdd={ctl.spec.newDocument ? () => void ctl.newDocument() : undefined}
                    addTitle={props.addTitle ?? `New tab (${keyLabel("ctrl+t")})`}
                    docDrag={{
                        docType: props.docType,
                        blockId: props.blockId,
                        canDrag: (id) => {
                            const tab = ctl.tabs().find((t) => t.id === id);
                            return !!tab && (props.canDrag?.(tab) ?? true);
                        },
                        onReorder: (id, targetId, position) => ctl.moveTo(id, targetId, position),
                    }}
                />
            </div>
        </Show>
    );
}
