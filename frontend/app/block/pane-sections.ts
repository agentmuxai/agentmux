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
    /** A section that is itself a row of sections (Warden's Vault): the selected one is in meta
     *  under `<section id>:section`. */
    sections?: PaneSections;
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
 *  entries and anything else that names a section by hand. `section` is an id,
 *  or `<id>/<sub id>` for a section of a section (`vault/cards` on Warden). */
export function sectionProblem(view: string | null | undefined, section: string): string | null {
    if (!view) return `"section": "${section}" needs a "view" in its blockdef`;
    const sections = paneSectionsOf(view);
    if (!sections) return `the ${view} pane has no sections`;
    // srv keys the section by the view as written; an old name's pane reads its own name's key.
    const name = resolvePaneTabView(view);
    if (name !== view) return `"${view}" is now the ${name} pane: use "view": "${name}"`;
    const [id, sub, ...rest] = section.split("/");
    if (!hasSection(sections, id)) {
        return `the ${name} pane has no "${id}" section (it has ${sections.items.map((s) => s.id).join(", ")})`;
    }
    if (sub === undefined) return null;
    const inner = sections.items.find((s) => s.id === id)?.sections;
    if (!inner || rest.length > 0) return `the ${name} pane's "${id}" section has no sections of its own`;
    if (!hasSection(inner, sub)) {
        return `the ${name} pane's "${id}" section has no "${sub}" (it has ${inner.items.map((s) => s.id).join(", ")})`;
    }
    return null;
}

/** The meta that opens `view` on `section` (an id, or `<id>/<sub id>`). */
export function sectionMetaOf(view: string, section: string): Record<string, string> {
    const [id, sub] = section.split("/");
    return sub === undefined ? { [sectionKeyOf(view)]: id } : { [sectionKeyOf(view)]: id, [sectionKeyOf(id)]: sub };
}

/** Whether `key: value` selects a section `view` declares: its own section key, or the key of a
 *  section's own sections. */
export function isSectionMeta(view: string, key: string, value: unknown): boolean {
    const sections = paneSectionsOf(view);
    if (!sections) return false;
    if (key === sectionKeyOf(view) && hasSection(sections, value)) return true;
    return sections.items.some((s) => !!s.sections && key === sectionKeyOf(s.id) && hasSection(s.sections, value));
}

/** The section keys in a blockdef's meta: what a widget-bar entry asks an open pane to show. */
export function sectionKeysIn(meta: Record<string, unknown> | null | undefined): Record<string, unknown> | undefined {
    const out = Object.fromEntries(Object.entries(meta ?? {}).filter(([k]) => k.endsWith(":section")));
    return Object.keys(out).length > 0 ? out : undefined;
}

/** The label of `section` on `view`, for a title or a menu. */
export function sectionLabelOf(view: string, section: string): string | undefined {
    return paneSectionsOf(view)?.items.find((s) => s.id === section)?.label;
}
