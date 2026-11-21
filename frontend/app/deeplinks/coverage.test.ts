// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The coverage test (it registers the app's pane views, whose modules need a DOM) (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md R13): every pane view the app registers
 * has a destination, every destination names a view that exists, every place and parameter has a label, and a
 * sectioned pane's sections are the registry's (until §5 moves the section lists into the registry).
 */

import { describe, expect, it } from "vitest";
import "@/app/block/block-registry";
import english from "@/app/i18n/locales/en.json";
import { DEFAULT_KEYBINDINGS } from "@/app/keybindings/defaults";
import { listPaneTabs, getPaneTab } from "@/app/store/pane-tab-registry";
import { DEEPLINKS, paneViewOf } from "./registry";
import { DEEPLINK_REGISTRY } from "./registry.generated";
import type { DeepLinkVar } from "./registry-types";

const known = new Set(Object.keys(english));

/** Every label key the registry holds, with where it is. */
function labelKeys(): [string, string][] {
    const out: [string, string][] = [];
    for (const view of DEEPLINKS.views) {
        out.push([view.labelKey, view.id]);
        for (const place of view.places) {
            const at = `${view.id}/${place.id}`;
            if (place.labelKey) out.push([place.labelKey, at]);
            for (const seg of place.segments) {
                if (typeof seg === "string") continue;
                for (const v of seg.values ?? []) out.push([v.labelKey, `${at} ${seg.var}=${v.id}`]);
            }
            if (place.anchor) out.push([place.anchor.labelKey, `${at}#`]);
            for (const p of place.params ?? []) out.push([p.labelKey, `${at}?${p.name}`]);
        }
    }
    return out;
}

describe("the deep-link registry covers the app (R13)", () => {
    const registered = listPaneTabs().filter((m) => !m.legacyOf);
    const linked = new Set(DEEPLINKS.views.filter((v) => !v.prefix).map(paneViewOf).filter((v): v is string => v !== null));
    const unlinked = new Map(DEEPLINKS.unlinked.map((u) => [u.view, u.reason]));

    it("reads the block registry (the list is not empty)", () => {
        expect(registered.length).toBeGreaterThan(15);
    });

    it("has a destination for every registered pane view, or says why not", () => {
        const missing = registered.map((m) => m.view).filter((v) => !linked.has(v) && !unlinked.has(v));
        expect(missing).toEqual([]);
    });

    it("names only views that exist", () => {
        const gone = [...linked, ...unlinked.keys()].filter((v) => !getPaneTab(v));
        expect(gone).toEqual([]);
    });

    it("does not both link and leave out a view", () => {
        expect([...unlinked.keys()].filter((v) => linked.has(v))).toEqual([]);
    });

    it("has a label in the English dictionary for every view, place, value, anchor and parameter", () => {
        const absent = labelKeys().filter(([key]) => !known.has(key)).map(([key, at]) => `${at}: ${key}`);
        expect(absent).toEqual([]);
    });

    it("gives every place that is not one enum segment its own label", () => {
        const bare = DEEPLINKS.views.flatMap((v) =>
            v.places.filter((p) => !p.labelKey && !(p.segments.length === 1 && typeof p.segments[0] !== "string" && p.segments[0].kind === "enum")).map((p) => `${v.id}/${p.id}`)
        );
        expect(bare).toEqual([]);
    });

    it("declares exactly the sections each sectioned pane has (R14)", () => {
        for (const view of DEEPLINKS.views) {
            for (const place of view.places.filter((p) => p.sections)) {
                const seg = place.segments.find((s): s is DeepLinkVar => typeof s !== "string" && s.kind === "enum")!;
                const pane = getPaneTab(paneViewOf(view));
                expect(pane?.sections, `${view.id} has sections`).toBeTruthy();
                expect(seg.values!.map((v) => v.id).sort(), view.id).toEqual(pane!.sections!.items.map((s) => s.id).sort());
            }
        }
    });

    it("names only command ids the shortcut table has (§4.1)", () => {
        const commands = new Set(DEFAULT_KEYBINDINGS.map((r) => r.command));
        const unknown = DEEPLINKS.views.filter((v) => v.command && !commands.has(v.command)).map((v) => `${v.id}: ${v.command}`);
        expect(unknown).toEqual([]);
    });

    it("is read by one import: the helpers hold the generated table itself (R11)", () => {
        expect(DEEPLINKS).toBe(DEEPLINK_REGISTRY);
    });

    it("lets an agent only reveal a worker (G1)", () => {
        const starters = DEEPLINKS.views.filter((v) => v.effect !== "none" && v.agent !== "reveal").map((v) => v.id);
        expect(starters).toEqual([]);
    });
});
