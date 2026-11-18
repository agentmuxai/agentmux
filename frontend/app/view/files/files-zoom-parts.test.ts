// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import en from "@/app/i18n/locales/en.json";
import { FILES_ZOOM, PLACES_PART, PLACES_ZOOM_KEY, seedPlacesZoom } from "./files-zoom-parts";

afterEach(() => vi.useRealTimers());

describe("Files' zoom parts", () => {
    it("declares Places as the one extra part, under its own key, and the rest as the pane's main level", () => {
        expect(FILES_ZOOM.parts).toEqual([
            { id: PLACES_PART, metaKey: "files:places_zoom", labelKey: "zoom.part.places", baseFontSize: 12 },
        ]);
        expect(PLACES_ZOOM_KEY).toBe("files:places_zoom");
        expect(FILES_ZOOM.mainLabelKey).toBe("zoom.part.files");
        expect(FILES_ZOOM.baseFontSize).toBe(12);
    });

    it("names every part with a key the dictionary has, so the indicator never shows a raw key", () => {
        const dictionary = en as Record<string, string>;
        const keys = [FILES_ZOOM.mainLabelKey, ...(FILES_ZOOM.parts ?? []).map((p) => p.labelKey)];
        expect(keys).toHaveLength(2);
        for (const key of keys) expect(dictionary[key as string], String(key)).toBeTruthy();
        expect([dictionary["zoom.part.places"], dictionary["zoom.part.files"]]).toEqual(["Places", "Files"]);
    });

    it("seeds Places from the pane's zoom once, so nobody's pane changes size", async () => {
        const persist = vi.fn();
        seedPlacesZoom({ "term:zoom": 1.3 }, persist);
        await Promise.resolve();
        expect(persist).toHaveBeenCalledTimes(1);
        expect(persist).toHaveBeenCalledWith({ "files:places_zoom": 1.3 });
    });

    it("seeds 100% when the pane has never been zoomed, so a later zoom of the rest cannot move Places", async () => {
        const persist = vi.fn();
        seedPlacesZoom({}, persist);
        seedPlacesZoom(undefined, persist);
        await Promise.resolve();
        expect(persist).toHaveBeenCalledTimes(2);
        expect(persist).toHaveBeenNthCalledWith(1, { "files:places_zoom": 1 });
    });

    it("never reseeds a pane that has the key", async () => {
        const persist = vi.fn();
        seedPlacesZoom({ "term:zoom": 2, "files:places_zoom": 1.1 }, persist);
        await Promise.resolve();
        expect(persist).not.toHaveBeenCalled();
    });
});
