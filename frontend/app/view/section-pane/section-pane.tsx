// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane with a row of sections, each one manager component: the shape of
 * the Connectors and Memory panes (connectors.tsx, memory.tsx). The selected section lives in block meta under
 * `<view>:section` (pane-sections.ts), so it names the pane, survives a remount, and lets an entry
 * point open the pane on a given section (panes.ts).
 */

import { createMemo, For, type Accessor, type JSX } from "solid-js";

import { sectionKeyOf, sectionOf, type PaneSections } from "@/app/block/pane-sections";
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
    /** Given the pane, for a section that needs its block (Remotes). */
    component: (pane: SectionPaneModel<Id>) => JSX.Element;
}

export interface SectionPaneSpec<Id extends string> {
    view: string;
    /** Section shown when the meta holds none, or an unknown id. */
    defaultSection: Id;
    /** The section tabs' accessible name. */
    ariaLabel: string;
    sections: readonly PaneSection<Id>[];
}

/** A spec from the sections a manifest declares (panes.ts) and each one's component. */
export function sectionPaneSpec<Id extends string>(
    view: string,
    ariaLabel: string,
    sections: PaneSections<Id>,
    components: Record<Id, PaneSection<Id>["component"]>
): SectionPaneSpec<Id> {
    return {
        view,
        defaultSection: sections.default,
        ariaLabel,
        sections: sections.items.map((info) => ({ ...info, component: components[info.id] })),
    };
}

/** The spec's sections as the pane's manifest declares them (`PaneTabManifest.sections`). */
export function sectionsOfSpec<Id extends string>(spec: SectionPaneSpec<Id>): PaneSections<Id> {
    return {
        default: spec.defaultSection,
        items: spec.sections.map(({ id, label, icon, tooltip }) => ({ id, label, icon, tooltip })),
    };
}

/** A section pane's state: the selected section and the zoom, both in block meta. */
export class SectionPaneModel<Id extends string> {
    viewType: string;
    blockId: string;
    spec: SectionPaneSpec<Id>;
    /** The block's meta, reactive. */
    meta: Accessor<MetaType | undefined>;
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
        this.meta = meta;
        this.zoomAtom = createMemo<number>(() => readZoom(meta()));
        const sections = sectionsOfSpec(spec);
        this.sectionAtom = createMemo<Id>(() => sectionOf(sections, spec.view, meta() as Record<string, unknown> | undefined));
        this.viewName = createMemo<string>(
            () => spec.sections.find((sec) => sec.id === this.sectionAtom())?.label ?? ""
        );
    }

    selectSection(id: Id): void {
        this.setMeta({ [sectionKeyOf(this.spec.view)]: id });
    }
}

export function SectionPaneView<Id extends string>(props: { model: SectionPaneModel<Id> }): JSX.Element {
    const model = props.model;
    const sections = model.spec.sections;
    const section = model.sectionAtom;
    let viewRef: HTMLDivElement | undefined;

    // Ctrl+scroll and Ctrl/Cmd +/-/0 zoom this pane through the shared handlers
    // (app.tsx, keymodel.ts): the `paneZoom` capability is all it needs.

    return (
        // The container carries container-type, so the managers inside can
        // answer the `section-pane` container queries; an element can't answer its own.
        <div class="section-pane-container">
            <div class="section-pane-view" ref={viewRef} style={{ zoom: model.zoomAtom() }}>
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
                                {item.component(model)}
                            </div>
                        )}
                    </For>
                </TabbedPane>
            </div>
        </div>
    );
}
