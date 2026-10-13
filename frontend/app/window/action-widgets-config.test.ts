// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import {
    buildPaneWidgetMenuItems,
    getChildWidgets,
    getEffectiveGroupedChildKeys,
    getGroupedChildKeys,
    getMoreWidgets,
    getHiddenKeys,
    getPinnedKeys,
    getPinnedWidgets,
    getWidgetBarModel,
    isWidgetHidden,
    nextWidgetBarConfig,
    withVisibleOrder,
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
    // Empty-tab menu / pane-tab picker — grouped children must not show up
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

// ── Hidden entries: SPEC_WIDGET_BAR_VISIBILITY_SETTINGS_2026_10_12.md ────────────

function barWmap(): Record<string, WidgetConfigType> {
    return {
        ...messengersWmap(),
        "defwidget@editor": widget({ "display:order": 2, "display:pinned": true, label: "Editor", blockdef: { meta: { view: "editor" } } }),
        "defwidget@drone": widget({ "display:order": 12, label: "Drone", blockdef: { meta: { view: "drone" } } }),
        "defwidget@settings": widget({ "display:order": 99, label: "Settings", blockdef: { meta: { view: "settings" } } }),
        "ext@acme.board/main": widget({ "display:order": 50, label: "Board", blockdef: { meta: { view: "ext:acme.board/main" } } }),
    };
}

describe("widget:hidden", () => {
    it("reads full keys and never hides Settings", () => {
        const hidden = getHiddenKeys({ "widget:hidden": ["defwidget@drone", "ext@acme.board/main", "defwidget@settings", 7] });
        expect(hidden).toEqual(new Set(["defwidget@drone", "ext@acme.board/main"]));
    });

    it("takes a hidden entry off the bar, out of More and out of the + picker", () => {
        const settings = { "widget:hidden": ["defwidget@editor", "defwidget@drone", "ext@acme.board/main"] };
        expect(getPinnedKeys(settings, barWmap())).toEqual(["agent"]);
        expect(getMoreWidgets(settings, barWmap()).map((e) => e.key)).not.toContain("defwidget@drone");
        expect(getMoreWidgets(settings, barWmap()).map((e) => e.key)).not.toContain("ext@acme.board/main");
        const labels = buildPaneWidgetMenuItems(barWmap(), settings, vi.fn()).map((i) => i.label);
        expect(labels).not.toContain("Editor");
        expect(labels).not.toContain("Drone");
        expect(labels).not.toContain("Board");
        expect(labels).toContain("Agent");
    });

    it("drops a hidden child from its group, and hides the group once every child is hidden", () => {
        const one = { "widget:hidden": ["defwidget@discord"] };
        expect(getChildWidgets(barWmap()["defwidget@messengers"], barWmap(), one).map((c) => c.key)).toEqual(["defwidget@slack"]);
        expect(isWidgetHidden("defwidget@messengers", barWmap(), one)).toBe(false);
        const both = { "widget:hidden": ["defwidget@discord", "defwidget@slack"] };
        expect(isWidgetHidden("defwidget@messengers", barWmap(), both)).toBe(true);
        expect(getMoreWidgets(both, barWmap()).map((e) => e.key)).not.toContain("defwidget@messengers");
        expect(buildPaneWidgetMenuItems(barWmap(), both, vi.fn()).map((i) => i.label)).not.toContain("Messengers");
    });
});

describe("nextWidgetBarConfig", () => {
    it("hiding a pinned entry unpins it in the same write", () => {
        expect(nextWidgetBarConfig("defwidget@editor", "hidden", {}, barWmap())).toEqual({
            "widget:hidden": ["defwidget@editor"],
            "widget:pinned": ["agent"],
        });
    });

    it("showing an entry again puts it in More and clears the list when it empties", () => {
        const settings = { "widget:hidden": ["defwidget@drone"] };
        expect(nextWidgetBarConfig("defwidget@drone", "more", settings, barWmap())).toEqual({ "widget:hidden": null });
    });

    it("pinning a hidden entry un-hides it and appends it to the pins", () => {
        const settings = { "widget:pinned": ["agent"], "widget:hidden": ["defwidget@drone", "ext@gone/x"] };
        expect(nextWidgetBarConfig("defwidget@drone", "pinned", settings, barWmap())).toEqual({
            "widget:hidden": ["ext@gone/x"],
            "widget:pinned": ["agent", "drone"],
        });
    });

    it("showing a group hidden only through its children brings the children back", () => {
        const settings = { "widget:hidden": ["defwidget@discord", "defwidget@slack", "defwidget@drone"] };
        expect(isWidgetHidden("defwidget@messengers", barWmap(), settings)).toBe(true);
        expect(nextWidgetBarConfig("defwidget@messengers", "more", settings, barWmap())).toEqual({ "widget:hidden": ["defwidget@drone"] });
        expect(nextWidgetBarConfig("defwidget@messengers", "pinned", settings, barWmap())).toEqual({
            "widget:hidden": ["defwidget@drone"],
            "widget:pinned": ["agent", "editor", "messengers"],
        });
    });

    it("keeps the pin of a group hidden only through its children when something else changes", () => {
        const settings = { "widget:pinned": ["agent", "messengers"], "widget:hidden": ["defwidget@discord", "defwidget@slack"] };
        expect(getPinnedKeys(settings, barWmap())).toEqual(["agent"]); // off the bar while its children are hidden
        expect(nextWidgetBarConfig("defwidget@agent", "hidden", settings, barWmap())).toEqual({
            "widget:hidden": ["defwidget@discord", "defwidget@slack", "defwidget@agent"],
            "widget:pinned": ["messengers"],
        });
        expect(nextWidgetBarConfig("defwidget@drone", "pinned", settings, barWmap())["widget:pinned"]).toEqual(["agent", "messengers", "drone"]);
    });

    it("reorders the visible pins without dropping a pin that is hidden right now", () => {
        expect(withVisibleOrder(["agent", "messengers", "editor"], ["editor", "agent"])).toEqual(["editor", "messengers", "agent"]);
    });

    it("refuses to hide Settings", () => {
        expect(nextWidgetBarConfig("defwidget@settings", "hidden", {}, barWmap())).toEqual({});
    });

});

describe("getWidgetBarModel", () => {
    it("lists pinned in pin order, then More, then hidden, with group children nested", () => {
        const settings = { "widget:pinned": ["editor", "agent"], "widget:hidden": ["defwidget@drone", "defwidget@slack"] };
        const model = getWidgetBarModel(settings, barWmap());
        expect(model.map((e) => [e.key, e.state])).toEqual([
            ["defwidget@editor", "pinned"],
            ["defwidget@agent", "pinned"],
            ["defwidget@help", "more"],
            ["defwidget@messengers", "more"],
            ["ext@acme.board/main", "more"],
            ["defwidget@settings", "more"],
            ["defwidget@drone", "hidden"],
        ]);
        const messengers = model.find((e) => e.key === "defwidget@messengers")!;
        expect(messengers.children.map((c) => [c.key, c.state])).toEqual([
            ["defwidget@discord", "group"],
            ["defwidget@slack", "hidden"],
        ]);
        expect(model.find((e) => e.key === "defwidget@settings")!.canHide).toBe(false);
    });

    it("gives every pinned entry, a promoted child too, its place on the bar", () => {
        const model = getWidgetBarModel({ "widget:pinned": ["slack", "agent"] }, barWmap());
        const agent = model.find((e) => e.key === "defwidget@agent")!;
        const slack = model.find((e) => e.key === "defwidget@messengers")!.children.find((c) => c.key === "defwidget@slack")!;
        expect([slack.state, slack.pinIndex, slack.pinCount]).toEqual(["pinned", 0, 2]);
        expect([agent.pinIndex, agent.pinCount]).toEqual([1, 2]);
        expect(model.find((e) => e.key === "ext@acme.board/main")!.canPin).toBe(false);
    });
});
