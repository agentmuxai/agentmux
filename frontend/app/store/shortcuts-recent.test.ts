// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Shortcuts pane's last-used order. docs/specs/SPEC_SHORTCUTS_PANE_FOR_SPLITS_2026_10_28.md §2.2. */

import { beforeEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.hoisted(() => vi.fn(async (..._args: unknown[]) => undefined));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: setConfig } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { setFullConfigAtom } from "@/app/store/config-signals";
import { orderByRecent, pushRecent, recordBlockOpened, SHORTCUTS_RECENT_MAX, widgetKeyForBlockDef } from "./shortcuts-recent";

const w = (view: string, extra: Record<string, unknown> = {}, more: Partial<WidgetConfigType> = {}): WidgetConfigType =>
    ({ label: view, blockdef: { meta: { view, ...extra } }, ...more }) as WidgetConfigType;
const widgets = (): Record<string, WidgetConfigType> => ({
    "defwidget@agent": w("agent"),
    "defwidget@terminal": w("term"),
    "defwidget@files": w("files"),
    "defwidget@vault": w("warden", { "warden:section": "vault" }),
    "defwidget@history": w("warden", { "warden:section": "history" }),
    "defwidget@messengers": w("messengers", {}, { children: ["discord"] }),
    "ext@acme.board/main": w("ext:acme.board/main"),
});

describe("pushRecent", () => {
    it("puts the key first, without repeating it", () => {
        expect(pushRecent(["a", "b", "c"], "b")).toEqual(["b", "a", "c"]);
        expect(pushRecent(undefined, "a")).toEqual(["a"]);
    });

    it(`keeps the newest ${SHORTCUTS_RECENT_MAX}`, () => {
        const keys = Array.from({ length: SHORTCUTS_RECENT_MAX }, (_, i) => `k${i}`);
        const next = pushRecent(keys, "new");
        expect(next).toHaveLength(SHORTCUTS_RECENT_MAX);
        expect(next[0]).toBe("new");
        expect(next).not.toContain(`k${SHORTCUTS_RECENT_MAX - 1}`);
    });
});

describe("orderByRecent", () => {
    const entries = [["a", 1], ["b", 2], ["c", 3], ["d", 4]] as const;

    it("a used entry comes first, in last-used order; unused ones keep their configured place", () => {
        expect(orderByRecent(entries, ["c", "a"]).map(([k]) => k)).toEqual(["c", "a", "b", "d"]);
    });

    it("no list changes nothing", () => {
        expect(orderByRecent(entries, undefined).map(([k]) => k)).toEqual(["a", "b", "c", "d"]);
        expect(orderByRecent(entries, []).map(([k]) => k)).toEqual(["a", "b", "c", "d"]);
    });

    it("a key that is not among the entries (hidden, uninstalled) is skipped, and nothing repeats", () => {
        expect(orderByRecent(entries, ["gone", "d", "d"]).map(([k]) => k)).toEqual(["d", "a", "b", "c"]);
    });
});

describe("widgetKeyForBlockDef", () => {
    it("finds the entry of a block's view", () => {
        expect(widgetKeyForBlockDef(widgets(), { meta: { view: "term", controller: "shell" } })).toBe("defwidget@terminal");
        expect(widgetKeyForBlockDef(widgets(), { meta: { view: "ext:acme.board/main" } })).toBe("ext@acme.board/main");
    });

    it("where entries share a view, the one whose own meta the block carries", () => {
        expect(widgetKeyForBlockDef(widgets(), { meta: { view: "warden", "warden:section": "history" } })).toBe("defwidget@history");
        expect(widgetKeyForBlockDef(widgets(), { meta: { view: "warden", "warden:section": "vault" } })).toBe("defwidget@vault");
    });

    it("is none for the Shortcuts pane itself, a group, a view with no entry or no view", () => {
        expect(widgetKeyForBlockDef(widgets(), { meta: { view: "shortcuts" } })).toBeUndefined();
        expect(widgetKeyForBlockDef(widgets(), { meta: { view: "launcher" } })).toBeUndefined();
        expect(widgetKeyForBlockDef(widgets(), { meta: { view: "messengers" } })).toBeUndefined();
        expect(widgetKeyForBlockDef(widgets(), { meta: { view: "unknown" } })).toBeUndefined();
        expect(widgetKeyForBlockDef(widgets(), { meta: {} })).toBeUndefined();
        expect(widgetKeyForBlockDef(undefined, { meta: { view: "term" } })).toBeUndefined();
    });
});

describe("recordBlockOpened", () => {
    beforeEach(() => {
        setConfig.mockClear();
        setFullConfigAtom({ settings: {}, widgets: widgets() } as unknown as FullConfigType);
    });

    it("writes the widget key first in launcher:recent", () => {
        recordBlockOpened({ meta: { view: "term" } });
        expect(setConfig).toHaveBeenCalledWith({}, { "launcher:recent": ["defwidget@terminal"] });
    });

    it("keeps the earlier ones behind it", () => {
        setFullConfigAtom({ settings: { "launcher:recent": ["defwidget@agent", "defwidget@files"] }, widgets: widgets() } as unknown as FullConfigType);
        recordBlockOpened({ meta: { view: "files" } });
        expect(setConfig).toHaveBeenCalledWith({}, { "launcher:recent": ["defwidget@files", "defwidget@agent"] });
    });

    it("writes nothing when it is first already, or for a block no entry stands for", () => {
        setFullConfigAtom({ settings: { "launcher:recent": ["defwidget@terminal"] }, widgets: widgets() } as unknown as FullConfigType);
        recordBlockOpened({ meta: { view: "term" } });
        recordBlockOpened({ meta: { view: "shortcuts" } });
        recordBlockOpened(undefined);
        expect(setConfig).not.toHaveBeenCalled();
    });

    it("never throws when the config is not there yet", () => {
        setFullConfigAtom(undefined as unknown as FullConfigType);
        expect(() => recordBlockOpened({ meta: { view: "term" } })).not.toThrow();
    });
});
