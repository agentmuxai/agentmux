// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The document-tab strip: a pane's documents, drawn with the shared
 * `PaneTabStrip` (as the Editor's file tabs are), under the pane header.
 * docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §4.1.
 *
 * Drag-reorder is not wired here: `PaneTabStrip`'s drag is a pane-tab drag
 * (it can tear the "tab" off into a window), which a document is not.
 * Reorder is by keys (Ctrl+Shift+PageUp/PageDown) until documents get a drag
 * type of their own (§5.8).
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
                    getTabClass={(t) => ({ "doc-tab--preview": t.preview, "doc-tab--pinned": t.pinned, "doc-tab--no-icon": !t.icon })}
                    onActivate={(id) => ctl.activate(id)}
                    onClose={(id) => void ctl.close(id)}
                    onTabDoubleClick={(t) => {
                        if (t.preview) ctl.promote(t.id);
                    }}
                    onAdd={ctl.spec.newDocument ? () => void ctl.newDocument() : undefined}
                    addTitle={props.addTitle ?? `New tab (${keyLabel("ctrl+t")})`}
                />
            </div>
        </Show>
    );
}
