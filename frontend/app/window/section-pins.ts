// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Pin this section to the widget bar": a sectioned pane's header menu
 * (blockframe.tsx) pins the section it shows, the way Kanban is pinned
 * (docs/reports/REPORT_PANE_SECTIONS_AS_WIDGETS_AND_VAULT_IN_WARDEN_2026_10_21.md §3.4).
 * The bar pins widget entries, so the section's entry is found (a built-in
 * like Kanban, or one the user wrote) or added to the user's widgets.json by
 * srv (`widgets.sectionentry`), then pinned. Unpinning leaves the entry in
 * More, like any other widget.
 */

import { movedMeta, paneSectionsOf, sectionKeysIn, sectionMetaOf, sectionOf } from "@/app/block/pane-sections";
import { resolvePaneTabView } from "@/app/block/pane-tab-registry";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { atoms } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { getPinnedKeys, pinIdOf, setWidgetBarState } from "./action-widgets-config";
import { MENU_ICON } from "@/app/util/menu-icons";

/** A section a pane shows, as a widget-bar entry would name it. */
export interface ShownSection {
    view: string;
    /** `<id>` or `<id>/<sub id>`. */
    path: string;
    label: string;
    icon: string;
}

/** The sections a pane shows now: its section, and that section's own when it has them
 *  (Warden on Vault ▸ Cards gives both). */
export function shownSections(meta: Record<string, unknown> | null | undefined): ShownSection[] {
    const raw = meta?.view;
    if (typeof raw !== "string") return [];
    const view = resolvePaneTabView(raw);
    const sections = paneSectionsOf(view);
    if (!sections) return [];
    const id = sectionOf(sections, view, meta);
    const item = sections.items.find((s) => s.id === id)!;
    const out: ShownSection[] = [{ view, path: id, label: item.label, icon: item.icon }];
    if (item.sections) {
        const subId = sectionOf(item.sections, id, meta);
        const sub = item.sections.items.find((s) => s.id === subId)!;
        out.push({ view, path: `${id}/${subId}`, label: sub.label, icon: sub.icon });
    }
    return out;
}

/** The widget entry that opens exactly this section, if there is one. */
export function sectionEntryKey(section: ShownSection, widgets: Record<string, WidgetConfigType>): string | undefined {
    const want = sectionMetaOf(section.view, section.path);
    const same = (got: Record<string, unknown> | undefined) =>
        !!got && Object.keys(got).length === Object.keys(want).length && Object.entries(want).every(([k, v]) => got[k] === v);
    return Object.entries(widgets).find(([, w]) => {
        // An entry that still names a moved section (Warden's Kanban) is the entry for where it lives now.
        const meta = movedMeta(w?.blockdef?.meta as Record<string, unknown> | undefined);
        return meta?.view === section.view && !w.children?.length && same(sectionKeysIn(meta));
    })?.[0];
}

/** Pins the section's entry, adding one to widgets.json when none opens it. Pinning also shows an
 *  entry the user had hidden, in the same write (setWidgetBarState), so it lands on the bar. */
export async function pinSection(section: ShownSection): Promise<void> {
    const config = atoms.fullConfigAtom();
    let key = sectionEntryKey(section, config?.widgets ?? {});
    if (!key) {
        key = (
            await RpcApi.WidgetsSectionEntryCommand(TabRpcClient, {
                view: section.view,
                section: section.path,
                label: section.label,
                icon: section.icon,
            })
        ).key;
    }
    await setWidgetBarState(key, "pinned", config?.settings ?? {}, atoms.fullConfigAtom()?.widgets ?? {});
}

/** The header menu's items for the sections a pane shows: pin each, or unpin it if pinned. */
export function sectionPinMenuItems(meta: Record<string, unknown> | null | undefined): ContextMenuItem[] {
    const config = atoms.fullConfigAtom();
    const widgets = config?.widgets ?? {};
    const settings = config?.settings ?? {};
    // On the bar now: a pinned but hidden entry offers to pin (which shows it), not to unpin.
    const pinned = new Set(getPinnedKeys(settings, widgets));
    return shownSections(meta).map((section) => {
        const key = sectionEntryKey(section, widgets);
        if (key && pinned.has(pinIdOf(key))) {
            return { label: `Unpin “${section.label}” from bar`, icon: MENU_ICON.unpin, click: () => fireAndForget(() => setWidgetBarState(key, "more", settings, widgets)) };
        }
        return { label: `Pin “${section.label}” to bar`, icon: MENU_ICON.pin, click: () => fireAndForget(() => pinSection(section)) };
    });
}
