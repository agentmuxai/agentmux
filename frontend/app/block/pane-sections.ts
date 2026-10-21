// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane's sections: a row of tabs, one shown at a time (Warden, the Vault,
 * Connectors, Memory, Settings). A pane declares them on its manifest
 * (`PaneTabManifest.sections`), and its selected section is always in block
 * meta under `<view>:section`. So anything that opens a pane can open it on a
 * section: a widget-bar entry (`"section"` in widgets.json), a palette command,
 * another pane, an agent, or a widget's `am.panes.open`
 * (docs/reports/REPORT_PANE_SECTIONS_AS_WIDGETS_AND_VAULT_IN_WARDEN_2026_10_21.md §3.4).
 *
 * No components here, so models and the widget bridge can import it.
 */

import { getPaneTab, resolvePaneTabView } from "./pane-tab-registry";

export interface PaneSectionInfo<Id extends string = string> {
    id: Id;
    label: string;
    /** Font Awesome name. */
    icon: string;
    tooltip?: string;
}

export interface PaneSections<Id extends string = string> {
    /** Shown when the meta names no section, or one the pane doesn't have. */
    default: Id;
    items: readonly PaneSectionInfo<Id>[];
}

/** The block meta key that holds `view`'s selected section. */
export function sectionKeyOf(view: string): string {
    return `${view}:section`;
}

export function hasSection<Id extends string>(sections: PaneSections<Id>, id: unknown): id is Id {
    return typeof id === "string" && sections.items.some((s) => s.id === id);
}

/** The section `meta` selects, or the pane's default. */
export function sectionOf<Id extends string>(
    sections: PaneSections<Id>,
    view: string,
    meta: Record<string, unknown> | null | undefined
): Id {
    const s = meta?.[sectionKeyOf(view)];
    return hasSection(sections, s) ? s : sections.default;
}

/** The sections a registered view declares (an old alias resolves to its view). */
export function paneSectionsOf(view: string | null | undefined): PaneSections | undefined {
    return getPaneTab(view)?.sections;
}

/** Why `section` can't open on `view`, or null when it can. For widgets.json
 *  entries and anything else that names a section by hand. */
export function sectionProblem(view: string | null | undefined, section: string): string | null {
    if (!view) return `"section": "${section}" needs a "view" in its blockdef`;
    const sections = paneSectionsOf(view);
    if (!sections) return `the ${view} pane has no sections`;
    if (!hasSection(sections, section)) {
        return `the ${resolvePaneTabView(view)} pane has no "${section}" section (it has ${sections.items.map((s) => s.id).join(", ")})`;
    }
    return null;
}

/** The label of `section` on `view`, for a title or a menu. */
export function sectionLabelOf(view: string, section: string): string | undefined {
    return paneSectionsOf(view)?.items.find((s) => s.id === section)?.label;
}
