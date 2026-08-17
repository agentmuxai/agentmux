// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    getChildWidgets,
    getGroupedChildKeys,
    getMoreWidgets,
    getPinnedKeys,
    getPinnedWidgets,
} from "./action-widgets-config";

function widget(overrides: Partial<WidgetConfigType> = {}): WidgetConfigType {
    return {
        "display:order": 0,
        "display:pinned": false,
        icon: "circle",
        label: "Widget",
        blockdef: { meta: { view: "browser" } },
        ...overrides,
    } as WidgetConfigType;
}

// Messengers fixture — the concrete first parent/children set from
// SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md.
function messengersWmap(): Record<string, WidgetConfigType> {
    return {
        "defwidget@agent": widget({ "display:order": 1, "display:pinned": true, label: "Agent" }),
        "defwidget@messengers": widget({
            "display:order": 10,
            "display:pinned": false,
            label: "Messengers",
            children: ["discord", "slack"],
            blockdef: { files: {}, meta: {} },
        }),
        "defwidget@discord": widget({ "display:order": 10, "display:hidden": true, label: "Discord" }),
        "defwidget@slack": widget({ "display:order": 11, "display:hidden": true, label: "Slack" }),
        "defwidget@help": widget({ "display:order": 9, label: "Help" }),
    };
}

describe("getGroupedChildKeys", () => {
    it("collects every short-name across all parents' children lists", () => {
        expect(getGroupedChildKeys(messengersWmap())).toEqual(new Set(["discord", "slack"]));
    });

    it("returns an empty set when nothing is grouped", () => {
        const wmap = { "defwidget@agent": widget() };
        expect(getGroupedChildKeys(wmap).size).toBe(0);
    });

    it("tolerates a missing/undefined wmap", () => {
        expect(getGroupedChildKeys(undefined as unknown as Record<string, WidgetConfigType>).size).toBe(0);
    });
});

describe("getChildWidgets", () => {
    it("resolves children in declared order", () => {
        const wmap = messengersWmap();
        const resolved = getChildWidgets(wmap["defwidget@messengers"], wmap);
        expect(resolved.map((c) => c.key)).toEqual(["defwidget@discord", "defwidget@slack"]);
    });

    it("skips a child short-name that doesn't resolve to a real widget", () => {
        const wmap = messengersWmap();
        const parent = widget({ children: ["discord", "does-not-exist"] });
        const resolved = getChildWidgets(parent, wmap);
        expect(resolved.map((c) => c.key)).toEqual(["defwidget@discord"]);
    });

    it("returns an empty array for a leaf widget (no children)", () => {
        expect(getChildWidgets(widget(), {})).toEqual([]);
    });
});

describe("grouped children excluded from pinned/more (display:pinned defaults)", () => {
    it("never appear in getPinnedWidgets even if individually display:pinned", () => {
        const wmap = messengersWmap();
        wmap["defwidget@discord"]["display:pinned"] = true;
        const pinned = getPinnedWidgets({}, wmap);
        expect(pinned.map((p) => p.key)).not.toContain("defwidget@discord");
    });

    it("never appear in getMoreWidgets", () => {
        const more = getMoreWidgets({}, messengersWmap());
        expect(more.map((m) => m.key)).not.toContain("defwidget@discord");
        expect(more.map((m) => m.key)).not.toContain("defwidget@slack");
    });

    it("the parent itself is present in exactly one of pinned/more, per its own pin state", () => {
        const wmap = messengersWmap();

        const unpinned = { pinned: getPinnedWidgets({}, wmap), more: getMoreWidgets({}, wmap) };
        expect(unpinned.pinned.map((p) => p.key)).not.toContain("defwidget@messengers");
        expect(unpinned.more.map((m) => m.key)).toContain("defwidget@messengers");

        const settings = { "widget:pinned": ["messengers"] };
        const pinned = { pinned: getPinnedWidgets(settings, wmap), more: getMoreWidgets(settings, wmap) };
        expect(pinned.pinned.map((p) => p.key)).toContain("defwidget@messengers");
        expect(pinned.more.map((m) => m.key)).not.toContain("defwidget@messengers");
    });
});

describe("grouped children excluded from pinned/more (widget:pinned override)", () => {
    it("a stale bare child short-name in widget:pinned is still excluded", () => {
        // A user's `widget:pinned` predating the grouping still lists "slack"
        // directly — getPinnedKeys must not let it through even though it's
        // present in the settings array (SPEC §5 open question 3).
        const wmap = messengersWmap();
        const settings = { "widget:pinned": ["agent", "slack"] };
        expect(getPinnedKeys(settings, wmap)).toEqual(["agent"]);
        expect(getMoreWidgets(settings, wmap).map((m) => m.key)).not.toContain("defwidget@slack");
    });
});
