// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one reader of the deep-link registry in the app (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md R10,
 * R11): the parser, the executor, the palette, the link component and the toasts read destinations here, never from
 * the generated table directly.
 */

import type { ParsedDeepLink } from "./parse";
import { DEEPLINK_REGISTRY } from "./registry.generated";
import type { DeepLinkPlace, DeepLinkRegistry, DeepLinkView } from "./registry-types";

/** The contract every opener is written against (the executor implements it; R11a). */
export type {
    CanOpen,
    DestinationRequest,
    OpenDestination,
    OpenOptions,
    OpenOutcome,
    OpenOutcomeKind,
    OpenReasonCode,
    OpenSource,
    OpenTarget,
    OpenWrappers,
} from "./open-types";

export const DEEPLINKS: DeepLinkRegistry = DEEPLINK_REGISTRY;

export function deepLinkView(id: string): DeepLinkView | undefined {
    return DEEPLINKS.views.find((v) => v.id === id);
}

export function deepLinkPlace(view: string, place: string | null): DeepLinkPlace | undefined {
    return place === null ? undefined : deepLinkView(view)?.places.find((p) => p.id === place);
}

/** The views this AgentMux opens now: a view of a later build is declared and opens nothing. */
export function openableViews(): DeepLinkView[] {
    return DEEPLINKS.views.filter((v) => v.build <= DEEPLINKS.build);
}

/** The pane view a destination opens (block `meta.view`), or null for one that is not a pane (the status bar). */
export function paneViewOf(view: DeepLinkView): string | null {
    return view.paneView === undefined ? view.id : view.paneView;
}

/** The label key of a link's place: its enum value's own label when the place is one enum segment
 *  (`settings.sections.recording`), else the place's label; null for the view as it is. */
export function placeLabelKey(link: Pick<ParsedDeepLink, "view" | "place" | "values">): string | null {
    const place = deepLinkPlace(link.view, link.place);
    if (!place) return null;
    if (place.labelKey) return place.labelKey;
    for (const seg of place.segments) {
        if (typeof seg !== "string" && seg.kind === "enum") {
            return seg.values?.find((v) => v.id === String(link.values[seg.var]))?.labelKey ?? null;
        }
    }
    return null;
}

/**
 * The words for a place, made by one function so the separator is one character everywhere (R10, R17):
 * "Settings › Recording". `translate` is the app's `t`, passed in so this module stays free of the i18n store.
 */
export function destinationLabel(link: Pick<ParsedDeepLink, "view" | "place" | "values">, translate: (key: string) => string): string {
    const view = deepLinkView(link.view);
    if (!view) return link.view;
    const placeKey = placeLabelKey(link);
    const words = [translate(view.labelKey), ...(placeKey ? [translate(placeKey)] : [])];
    return words.join(` ${DEEPLINKS.separator} `);
}
