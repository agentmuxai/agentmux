// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane with a rail of sections, each one manager component: the shape of
 * the Connectors and Knowledge panes (connectors.tsx, knowledge.tsx), which
 * replaced the Armory. The selected section lives in block meta under the
 * pane's own key, so it names the pane, survives a remount, and lets an entry
 * point open the pane on a given section (panes.ts).
 */

import { createMemo, For, onCleanup, onMount, type Accessor, type JSX } from "solid-js";

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { Tooltip } from "@/app/element/tooltip";
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
    /** Section shown when the meta holds none, or an unknown id. */
    defaultSection: Id;
    /** The rail's accessible name. */
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
            const s = (meta() as Record<string, unknown> | undefined)?.[spec.sectionKey];
            return spec.sections.some((sec) => sec.id === s) ? (s as Id) : spec.defaultSection;
        });
        this.viewName = createMemo<string>(
            () => spec.sections.find((sec) => sec.id === this.sectionAtom())?.label ?? ""
        );
    }

    selectSection(id: Id): void {
        this.setMeta({ [this.spec.sectionKey]: id });
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
        // The container carries container-type, so .armory-view inside it can
        // answer the `armory` container queries; an element can't answer its own.
        <div class="armory-container">
            <div class="armory-view" ref={viewRef} style={{ zoom: model.zoomAtom() }}>
                {/* The narrow-width stand-in for the rail, first so it sits at the
                    top (SPEC_RESPONSIVE_TAB_BAR_TOP_POSITION_2026_08_24.md). */}
                <nav class="bundle-manager-tab-bar" aria-label={model.spec.ariaLabel}>
                    <For each={sections}>
                        {(item) => (
                            <button
                                type="button"
                                classList={{ "is-active": section() === item.id, "is-abf-highlight": !!item.highlight }}
                                aria-pressed={section() === item.id}
                                onClick={() => model.selectSection(item.id)}
                            >
                                <i class={`fa-sharp fa-solid fa-${item.icon}`} aria-hidden="true" />
                                <span>{item.label}</span>
                            </button>
                        )}
                    </For>
                </nav>
                <nav class="bundle-manager-rail" aria-label={model.spec.ariaLabel}>
                    <For each={sections}>
                        {(item) => (
                            <Tooltip content={item.tooltip ?? item.label} placement="right">
                                <button
                                    type="button"
                                    class="bundle-manager-rail-item"
                                    classList={{ "is-active": section() === item.id, "is-abf-highlight": !!item.highlight }}
                                    aria-pressed={section() === item.id}
                                    onClick={() => model.selectSection(item.id)}
                                >
                                    <i class={`fa-sharp fa-solid fa-${item.icon}`} aria-hidden="true" />
                                    <span>{item.label}</span>
                                </button>
                            </Tooltip>
                        )}
                    </For>
                </nav>
                <div class="bundle-manager-section">
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
                </div>
            </div>
        </div>
    );
}
