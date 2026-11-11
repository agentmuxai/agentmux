// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({ lang: "en" }));
vi.mock("@/app/i18n/i18n", () => ({
    t: (key: string) => `${state.lang}:${key}`,
}));

import { SETTINGS_SECTION_LABELS, SETTINGS_SECTIONS, settingEntry } from "./settings-model";

describe("settingEntry", () => {
    const entry = settingEntry({
        id: "x.y",
        section: "appearance",
        labelKey: "settings.x.y.label",
        descriptionKey: "settings.x.y.description",
        groupKey: "settings.x.group.z",
        keywords: ["synonym", "x:key"],
    });

    it("reads its label, description and group from the dictionary each time they are read", () => {
        expect(entry.label).toBe("en:settings.x.y.label");
        expect(entry.description).toBe("en:settings.x.y.description");
        expect(entry.group).toBe("en:settings.x.group.z");
        state.lang = "ja";
        expect(entry.label).toBe("ja:settings.x.y.label");
        expect(entry.group).toBe("ja:settings.x.group.z");
        state.lang = "en";
    });

    it("keeps the id, section and keywords as written, so a search by the English synonym still finds it", () => {
        expect(entry.id).toBe("x.y");
        expect(entry.section).toBe("appearance");
        expect(entry.keywords).toEqual(["synonym", "x:key"]);
    });

    it("has no description or group when none is named", () => {
        const bare = settingEntry({ id: "a.b", section: "advanced", labelKey: "settings.a.b.label", keywords: ["k"] });
        expect(bare.description).toBeUndefined();
        expect(bare.group).toBeUndefined();
    });
});

describe("the section labels", () => {
    it("follow the language, in the labels and in the rail's items", () => {
        const item = SETTINGS_SECTIONS.items.find((i) => i.id === "sounds")!;
        expect(SETTINGS_SECTION_LABELS.sounds).toBe("en:settings.sections.sounds");
        expect(item.label).toBe("en:settings.sections.sounds");
        state.lang = "ja";
        expect(item.label).toBe("ja:settings.sections.sounds");
        state.lang = "en";
    });
});
