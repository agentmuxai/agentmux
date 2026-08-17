// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import {
    buildPaneWidgetMenuItems,
    getChildWidgets,
    getEffectiveGroupedChildKeys,
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

    it("is unaffected by pin state — purely structural", () => {
        const wmap = messengersWmap();
        const settings = { "widget:pinned": ["slack"] };
        expect(getGroupedChildKeys(wmap)).toEqual(new Set(["discord", "slack"]));
        void settings; // getGroupedChildKeys doesn't take settings at all
    });
});

describe("getEffectiveGroupedChildKeys", () => {
    it("matches getGroupedChildKeys when nothing is individually pinned", () => {
        const wmap = messengersWmap();
        expect(getEffectiveGroupedChildKeys(wmap, {})).toEqual(new Set(["discord", "slack"]));
    });

    it("drops a child that's individually pinned via widget:pinned — it's been promoted out", () => {
        const wmap = messengersWmap();
        const settings = { "widget:pinned": ["agent", "slack"] };
        expect(getEffectiveGroupedChildKeys(wmap, settings)).toEqual(new Set(["discord"]));
    });

    it("re-includes a child once it's removed from widget:pinned (unpinned)", () => {
        const wmap = messengersWmap();
        expect(getEffectiveGroupedChildKeys(wmap, { "widget:pinned": ["agent"] })).toEqual(
            new Set(["discord", "slack"])
        );
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

    it("without settings, includes every declared child regardless of pin state", () => {
        const wmap = messengersWmap();
        const resolved = getChildWidgets(wmap["defwidget@messengers"], wmap);
        expect(resolved.map((c) => c.key)).toEqual(["defwidget@discord", "defwidget@slack"]);
    });

    it("with settings, excludes a child that's been individually pinned (promoted out)", () => {
        const wmap = messengersWmap();
        const settings = { "widget:pinned": ["slack"] };
        const resolved = getChildWidgets(wmap["defwidget@messengers"], wmap, settings);
        expect(resolved.map((c) => c.key)).toEqual(["defwidget@discord"]);
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

describe("individually pinning a grouped child (promote out of the group)", () => {
    it("a child explicitly listed in widget:pinned is honored — it shows as a standalone pinned widget", () => {
        // This is the actual feature: right-clicking "Slack" inside the
        // Messengers flyout and choosing "Pin to bar" writes "slack" into
        // widget:pinned exactly like pinning any other widget. It must NOT
        // be stripped back out.
        const wmap = messengersWmap();
        const settings = { "widget:pinned": ["agent", "slack"] };
        expect(getPinnedKeys(settings, wmap)).toEqual(["agent", "slack"]);
        expect(getPinnedWidgets(settings, wmap).map((p) => p.key)).toContain("defwidget@slack");
    });

    it("a promoted child never also appears in getMoreWidgets", () => {
        const wmap = messengersWmap();
        const settings = { "widget:pinned": ["slack"] };
        expect(getMoreWidgets(settings, wmap).map((m) => m.key)).not.toContain("defwidget@slack");
    });

    it("a promoted child is dropped from its parent's own resolved children (getChildWidgets + settings)", () => {
        const wmap = messengersWmap();
        const settings = { "widget:pinned": ["slack"] };
        const resolved = getChildWidgets(wmap["defwidget@messengers"], wmap, settings);
        expect(resolved.map((c) => c.key)).toEqual(["defwidget@discord"]);
    });

    it("a non-promoted sibling stays reachable only through the group", () => {
        const wmap = messengersWmap();
        const settings = { "widget:pinned": ["slack"] };
        expect(getPinnedKeys(settings, wmap)).not.toContain("discord");
        expect(getMoreWidgets(settings, wmap).map((m) => m.key)).not.toContain("defwidget@discord");
    });

    it("unpinning reverts the child to living only inside its parent group, regardless of the parent's own pin state", () => {
        const wmap = messengersWmap();
        // Slack was pinned, now it's unpinned (removed from widget:pinned) —
        // "goes back into Messengers" — this holds whether Messengers itself
        // is pinned or not.
        for (const settings of [{ "widget:pinned": [] as string[] }, { "widget:pinned": ["messengers"] }]) {
            expect(getPinnedKeys(settings, wmap)).not.toContain("slack");
            expect(getMoreWidgets(settings, wmap).map((m) => m.key)).not.toContain("defwidget@slack");
            expect(getChildWidgets(wmap["defwidget@messengers"], wmap, settings).map((c) => c.key)).toContain(
                "defwidget@slack"
            );
        }
    });
});

describe("buildPaneWidgetMenuItems", () => {
    // Replace With... / empty-tab menu — grouped children must not show up
    // individually, but must still be reachable via a nested submenu under
    // their parent's own label.
    function paneWmap(): Record<string, WidgetConfigType> {
        return {
            ...messengersWmap(),
            "defwidget@devtools": widget({ label: "DevTools", blockdef: { meta: { view: "devtools" } } }),
            "defwidget@editor": widget({ "display:order": 6, label: "Editor", blockdef: { meta: { view: "editor" } } }),
        };
    }

    it("excludes grouped children as flat top-level entries", () => {
        const items = buildPaneWidgetMenuItems(paneWmap(), {}, vi.fn());
        expect(items.map((i) => i.label)).not.toContain("Discord");
        expect(items.map((i) => i.label)).not.toContain("Slack");
    });

    it("nests a parent's children under its own label as a native submenu", () => {
        const items = buildPaneWidgetMenuItems(paneWmap(), {}, vi.fn());
        const messengers = items.find((i) => i.label === "Messengers");
        expect(messengers?.type).toBe("submenu");
        expect(messengers?.submenu?.map((c) => c.label)).toEqual(["Discord", "Slack"]);
    });

    it("excludes non-pane views (devtools) everywhere, including inside a submenu", () => {
        const items = buildPaneWidgetMenuItems(paneWmap(), {}, vi.fn());
        expect(items.map((i) => i.label)).not.toContain("DevTools");
    });

    it("excludes the current view via opts.excludeView (leaf) and from inside a submenu", () => {
        const wmap = paneWmap();
        wmap["defwidget@discord"].blockdef = { meta: { view: "editor" } };
        const items = buildPaneWidgetMenuItems(wmap, {}, vi.fn(), { excludeView: "editor" });
        expect(items.map((i) => i.label)).not.toContain("Editor");
        const messengers = items.find((i) => i.label === "Messengers");
        expect(messengers?.submenu?.map((c) => c.label)).toEqual(["Slack"]);
    });

    it("omits a parent entirely once every child is filtered out", () => {
        const wmap = paneWmap();
        wmap["defwidget@discord"].blockdef = { meta: { view: "devtools" } };
        wmap["defwidget@slack"].blockdef = { meta: { view: "devtools" } };
        const items = buildPaneWidgetMenuItems(wmap, {}, vi.fn());
        expect(items.map((i) => i.label)).not.toContain("Messengers");
    });

    it("invokes onSelect with the chosen widget's blockdef", () => {
        const onSelect = vi.fn();
        const items = buildPaneWidgetMenuItems(paneWmap(), {}, onSelect);
        const editor = items.find((i) => i.label === "Editor")!;
        editor.click?.();
        expect(onSelect).toHaveBeenCalledWith(paneWmap()["defwidget@editor"].blockdef);
    });

    it("a promoted child shows as a flat entry instead of nested under its parent", () => {
        const settings = { "widget:pinned": ["slack"] };
        const items = buildPaneWidgetMenuItems(paneWmap(), settings, vi.fn());
        expect(items.map((i) => i.label)).toContain("Slack");
        const messengers = items.find((i) => i.label === "Messengers");
        expect(messengers?.submenu?.map((c) => c.label)).toEqual(["Discord"]);
    });
});
