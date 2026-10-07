// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane with a row of sections, each one manager component: the shape of
 * the Connectors and Memory panes (connectors.tsx, memory.tsx), which
 * replaced the Armory. The selected section lives in block meta under the
 * pane's own key, so it names the pane, survives a remount, and lets an entry
 * point open the pane on a given section (panes.ts).
 */

import { createMemo, For, onCleanup, onMount, type Accessor, type JSX } from "solid-js";

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { TabbedPane } from "@/app/element/ui";
import { readZoom } from "@/app/store/zoom-factor";
import "./section-pane.scss";

export interface PaneSection<Id extends string> {
    id: Id;
    label: string;
    icon: string;
    tooltip?: string;
    /** A standing accent on the section's icon (Bundles, for ABF). */
    highlight?: boolean;
    component: () => JSX.Element;
}

export interface SectionPaneSpec<Id extends string> {
    view: string;
    /** Block meta key holding the selected section id. */
    sectionKey: string;
    /** Keys that held it before the pane was renamed, read when `sectionKey`
     *  holds nothing and cleared on the next selection. */
    legacySectionKeys?: readonly string[];
    /** Section shown when the meta holds none, or an unknown id. */
    defaultSection: Id;
    /** The section tabs' accessible name. */
    ariaLabel: string;
    sections: readonly PaneSection<Id>[];
}

/** A section pane's state: the selected section and the zoom, both in block meta. */
export class SectionPaneModel<Id extends string> {
    viewType: string;
    blockId: string;
    spec: SectionPaneSpec<Id>;
    setMeta: (patch: Record<string, unknown>) => void;
    /** Per-pane zoom, the same `term:zoom` key as the editor, terminal and agent. */
    zoomAtom: Accessor<number>;
    sectionAtom: Accessor<Id>;
    /** The selected section's label, which names the pane. */
    viewName: Accessor<string>;

    constructor(ctx: PaneTabHostContext, spec: SectionPaneSpec<Id>) {
        this.viewType = spec.view;
        this.blockId = ctx.blockId;
        this.spec = spec;
        this.setMeta = (patch) => void ctx.setMeta(patch);
        const meta = ctx.meta;
        this.zoomAtom = createMemo<number>(() => readZoom(meta()));
        this.sectionAtom = createMemo<Id>(() => {
            const m = meta() as Record<string, unknown> | undefined;
            const s = [spec.sectionKey, ...(spec.legacySectionKeys ?? [])].map((k) => m?.[k]).find((v) => v != null);
            return spec.sections.some((sec) => sec.id === s) ? (s as Id) : spec.defaultSection;
        });
        this.viewName = createMemo<string>(
            () => spec.sections.find((sec) => sec.id === this.sectionAtom())?.label ?? ""
        );
    }

    selectSection(id: Id): void {
        const cleared = Object.fromEntries((this.spec.legacySectionKeys ?? []).map((k) => [k, null]));
        this.setMeta({ ...cleared, [this.spec.sectionKey]: id });
    }
}

export function SectionPaneView<Id extends string>(props: { model: SectionPaneModel<Id> }): JSX.Element {
    const model = props.model;
    const sections = model.spec.sections;
    const section = model.sectionAtom;
    let viewRef: HTMLDivElement | undefined;

    // Ctrl+Wheel zooms this pane, as in the editor (editor-view.tsx). Capture
    // phase, so it runs before anything scrollable inside; preventDefault stops
    // CEF's own page zoom. Ctrl+Shift+Wheel is the all-panes gesture (app.tsx),
    // so it's left to bubble.
    onMount(() => {
        if (!viewRef) return;
        const handleCtrlWheel = (ev: WheelEvent) => {
            if (!ev.ctrlKey || ev.shiftKey) return;
            ev.preventDefault();
            ev.stopPropagation();
            const STEP = 0.1;
            const current = model.zoomAtom();
            const next = Math.max(0.5, Math.min(2.0, Math.round((current + (ev.deltaY > 0 ? -STEP : STEP)) * 100) / 100));
            model.setMeta({ "term:zoom": next === 1.0 ? null : next });
        };
        viewRef.addEventListener("wheel", handleCtrlWheel, { passive: false, capture: true });
        onCleanup(() => viewRef?.removeEventListener("wheel", handleCtrlWheel, { capture: true }));
    });

    return (
        // The container carries container-type, so the managers inside can
        // answer the `armory` container queries; an element can't answer its own.
        <div class="armory-container">
            <div class="armory-view" ref={viewRef} style={{ zoom: model.zoomAtom() }}>
                {/* One tablist along the top: icons only when narrow, labels when
                    there's room (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5.4). */}
                <TabbedPane
                    items={sections.map((item) => ({
                        id: item.id,
                        label: item.label,
                        icon: item.icon,
                        tooltip: item.tooltip,
                        class: item.highlight ? "is-abf-highlight" : undefined,
                    }))}
                    value={section()}
                    onChange={(id) => model.selectSection(id)}
                    ariaLabel={model.spec.ariaLabel}
                    panelClass="bundle-manager-section"
                >
                    {/* Every section stays mounted, so switching is instant and never
                        refetches; the managers keep themselves current from MPS
                        `*:changed` events. */}
                    <For each={sections}>
                        {(item) => (
                            <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== item.id }}>
                                {item.component()}
                            </div>
                        )}
                    </For>
                </TabbedPane>
            </div>
        </div>
    );
}
