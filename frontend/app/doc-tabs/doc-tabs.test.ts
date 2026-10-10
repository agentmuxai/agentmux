// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import {
    closeDoc,
    closeOthers,
    closeToRight,
    cycleDoc,
    emptyDocTabs,
    hydrateDocTabs,
    MAX_CLOSED,
    moveDoc,
    moveDocTo,
    openDoc,
    persistDocTabs,
    reopenClosed,
    setPinned,
    type DocTabsState,
} from "./doc-tabs";
import { DocTabsController, docTabKeyAction, handleDocTabKey, type DocTabsSpec } from "./doc-tabs-controller";

type P = { path: string };
const open = (s: DocTabsState<P>, path: string, extra: { preview?: boolean; activate?: boolean } = {}, id = path) =>
    openDoc(s, { key: path, title: path, payload: { path }, ...extra }, id);
const keys = (s: DocTabsState<P>) => s.tabs.map((t) => t.key);

describe("document tabs: the model", () => {
    it("opens after the active tab, and an open key just activates its tab", () => {
        let s = open(emptyDocTabs<P>(), "a");
        s = open(s, "b");
        s = { ...s, activeId: "a" };
        s = open(s, "c");
        expect(keys(s)).toEqual(["a", "c", "b"]);
        expect(s.activeId).toBe("c");
        const again = open(s, "b", {}, "other-id");
        expect(again.tabs).toHaveLength(3);
        expect(again.activeId).toBe("b");
    });

    it("a preview replaces the previous preview in place; a normal open keeps it", () => {
        let s = open(emptyDocTabs<P>(), "a");
        s = open(s, "p1", { preview: true });
        s = open(s, "p2", { preview: true });
        expect(keys(s)).toEqual(["a", "p2"]);
        s = open(s, "p2");
        expect(s.tabs[1].preview).toBe(false);
        s = open(s, "p3", { preview: true });
        expect(keys(s)).toEqual(["a", "p2", "p3"]);
    });

    it("closing activates the tab used before, and goes on the reopen list", () => {
        let s = open(open(open(emptyDocTabs<P>(), "a"), "b"), "c");
        s = { ...s, activeId: "a", mru: ["a", "c", "b"] };
        s = closeDoc(s, "a");
        expect(s.activeId).toBe("c");
        expect(s.closed.map((t) => t.key)).toEqual(["a"]);
        s = reopenClosed(s, "a2");
        expect(keys(s)).toContain("a");
        expect(s.activeId).toBe("a2");
        expect(s.closed).toHaveLength(0);
    });

    it("keepOne refuses to close the last tab; a closed preview isn't reopenable", () => {
        const one = open(emptyDocTabs<P>(), "a");
        expect(closeDoc(one, "a", { keepOne: true })).toBe(one);
        const p = closeDoc(open(one, "p", { preview: true }), "p");
        expect(p.closed).toHaveLength(0);
    });

    it("remembers at most ten closed tabs", () => {
        let s = emptyDocTabs<P>();
        for (let i = 0; i < 15; i++) s = open(s, `f${i}`);
        for (let i = 0; i < 14; i++) s = closeDoc(s, `f${i}`);
        expect(s.closed).toHaveLength(MAX_CLOSED);
    });

    it("keeps pinned tabs first, and moves within a group", () => {
        let s = open(open(open(emptyDocTabs<P>(), "a"), "b"), "c");
        s = setPinned(s, "c", true);
        expect(keys(s)).toEqual(["c", "a", "b"]);
        s = moveDoc(s, "b", -5);
        expect(keys(s)).toEqual(["c", "b", "a"]);
        s = moveDoc(s, "c", 1);
        expect(keys(s)).toEqual(["c", "b", "a"]);
    });

    it("moves a dragged tab before or after another", () => {
        const s = open(open(open(open(emptyDocTabs<P>(), "a"), "b"), "c"), "d");
        expect(keys(moveDocTo(s, "d", "a", "before"))).toEqual(["d", "a", "b", "c"]);
        expect(keys(moveDocTo(s, "a", "c", "after"))).toEqual(["b", "c", "a", "d"]);
        expect(keys(moveDocTo(s, "a", "c", "before"))).toEqual(["b", "a", "c", "d"]);
    });

    it("a drag that lands where the tab already is changes nothing", () => {
        const s = open(open(open(emptyDocTabs<P>(), "a"), "b"), "c");
        expect(moveDocTo(s, "b", "a", "after")).toBe(s);
        expect(moveDocTo(s, "b", "b", "before")).toBe(s);
        expect(moveDocTo(s, "nope", "a", "before")).toBe(s);
        expect(moveDocTo(s, "a", "nope", "before")).toBe(s);
    });

    it("a dragged tab stays in its pinned or unpinned group", () => {
        let s = open(open(open(emptyDocTabs<P>(), "a"), "b"), "c");
        s = setPinned(s, "a", true);
        // Unpinned "c" dropped before pinned "a": first of the unpinned.
        expect(keys(moveDocTo(s, "c", "a", "before"))).toEqual(["a", "c", "b"]);
        // Pinned "a" dropped after unpinned "c": still the pinned group, first.
        expect(keys(moveDocTo(s, "a", "c", "after"))).toEqual(["a", "b", "c"]);
    });

    it("closes others and to the right, sparing pinned tabs", () => {
        let s = open(open(open(open(emptyDocTabs<P>(), "a"), "b"), "c"), "d");
        s = setPinned(s, "a", true);
        expect(keys(closeOthers(s, "c"))).toEqual(["a", "c"]);
        expect(keys(closeToRight(s, "b"))).toEqual(["a", "b"]);
    });

    it("cycles with wrap-around", () => {
        let s = open(open(open(emptyDocTabs<P>(), "a"), "b"), "c");
        s = cycleDoc(s, 1);
        expect(s.activeId).toBe("a");
        s = cycleDoc(s, -1);
        expect(s.activeId).toBe("c");
    });

    it("persists without ids, previews or the closed list, and restores", () => {
        let s = open(open(emptyDocTabs<P>(), "a"), "b");
        s = setPinned(s, "b", true);
        s = open(s, "p", { preview: true, activate: false });
        s = closeDoc(open(s, "x"), "x");
        s = { ...s, activeId: "a" };
        const saved = persistDocTabs(s, (p) => p.path);
        expect(saved).toEqual({
            v: 1,
            active: 1,
            tabs: [
                { key: "b", title: "b", pinned: true, state: "b" },
                { key: "a", title: "a", state: "a" },
            ],
        });
        const back = hydrateDocTabs<P>(JSON.parse(JSON.stringify(saved)), (st) => (typeof st === "string" ? { path: st } : null))!;
        expect(keys(back)).toEqual(["b", "a"]);
        expect(back.tabs.find((t) => t.id === back.activeId)?.key).toBe("a");
        expect(hydrateDocTabs({ v: 2, tabs: [] }, () => null)).toBeNull();
        expect(hydrateDocTabs({ v: 1, active: 0, tabs: [{ key: "a", title: "a", state: 1 }] }, () => null)).toBeNull();
    });
});

describe("document tabs: the controller", () => {
    afterEach(() => vi.useRealTimers());

    const spec: DocTabsSpec<P> = {
        keyOf: (p) => p.path,
        titleOf: (p) => p.path.split("/").pop() ?? p.path,
        serialize: (p) => p.path,
        deserialize: (st) => (typeof st === "string" ? { path: st } : null),
        newDocument: (active) => (active ? { path: active.path } : null),
        keepOne: true,
    };
    const host = () => {
        const meta: Record<string, unknown> = {};
        return { meta: () => meta, setMeta: (patch: Record<string, unknown>) => void Object.assign(meta, patch), raw: meta };
    };

    it("saves to block meta after a pause, and a new controller restores it", () => {
        vi.useFakeTimers();
        const h = host();
        const ctl = new DocTabsController(spec, h, [{ path: "/a" }]);
        ctl.open({ path: "/b" });
        expect(h.raw.doctabs).toBeUndefined();
        vi.advanceTimersByTime(300);
        expect((h.raw.doctabs as { tabs: unknown[] }).tabs).toHaveLength(2);
        const again = new DocTabsController(spec, h, [{ path: "/ignored" }]);
        expect(again.tabs().map((t) => t.key)).toEqual(["/a", "/b"]);
        expect(again.active()?.key).toBe("/b");
    });

    it("moves a dragged tab, and says whether anything moved", () => {
        const ctl = new DocTabsController(spec, host(), [{ path: "/a" }, { path: "/b" }, { path: "/c" }]);
        const [a, , c] = ctl.tabs().map((t) => t.id);
        expect(ctl.moveTo(c, a, "before")).toBe(true);
        expect(ctl.tabs().map((t) => t.key)).toEqual(["/c", "/a", "/b"]);
        expect(ctl.moveTo(c, a, "before")).toBe(false);
    });

    it("refuses to close a keepOne pane's last tab, and says so to the key handler", () => {
        const ctl = new DocTabsController(spec, host(), [{ path: "/a" }]);
        const refused = vi.fn();
        const e = new KeyboardEvent("keydown", { key: "w", ctrlKey: true });
        expect(handleDocTabKey(e, ctl, refused)).toBe(true);
        expect(refused).toHaveBeenCalled();
        expect(ctl.tabs()).toHaveLength(1);
    });

    it("Ctrl+T opens a second tab on the same document; Ctrl+W closes it", () => {
        const ctl = new DocTabsController(spec, host(), [{ path: "/a" }]);
        handleDocTabKey(new KeyboardEvent("keydown", { key: "t", ctrlKey: true }), ctl);
        expect(ctl.tabs()).toHaveLength(2);
        handleDocTabKey(new KeyboardEvent("keydown", { key: "w", ctrlKey: true }), ctl);
        expect(ctl.tabs()).toHaveLength(1);
        handleDocTabKey(new KeyboardEvent("keydown", { key: "T", ctrlKey: true, shiftKey: true }), ctl);
        expect(ctl.tabs()).toHaveLength(2);
    });

    it("says whether a tab was reopened (muxreview on #4231)", () => {
        const ctl = new DocTabsController(spec, host(), [{ path: "/a" }]);
        expect(ctl.reopen()).toBe(false);
        ctl.open({ path: "/b" });
        ctl.close(ctl.activeId()!);
        expect(ctl.reopen()).toBe(true);
        expect(ctl.active()?.key).toBe("/b");
    });

    it("tells the pane when a tab is gone for good", () => {
        const ctl = new DocTabsController(spec, host(), [{ path: "/a" }]);
        const gone = vi.fn();
        ctl.onClosed = gone;
        ctl.open({ path: "/p" }, { preview: true });
        ctl.open({ path: "/q" }, { preview: true });
        expect(gone.mock.calls.map((c) => c[0].key)).toEqual(["/p"]);
    });

    it("maps the keys, and leaves Cmd and Alt chords to the app", () => {
        expect(docTabKeyAction({ key: "Tab", ctrlKey: true, shiftKey: true, altKey: false, metaKey: false })).toEqual({ kind: "cycle", delta: -1 });
        expect(docTabKeyAction({ key: "PageUp", ctrlKey: true, shiftKey: true, altKey: false, metaKey: false })).toEqual({ kind: "move", delta: -1 });
        expect(docTabKeyAction({ key: "w", ctrlKey: true, shiftKey: false, altKey: true, metaKey: false })).toBeNull();
        expect(docTabKeyAction({ key: "w", ctrlKey: false, shiftKey: false, altKey: false, metaKey: true })).toBeNull();
        expect(docTabKeyAction({ key: "f", ctrlKey: true, shiftKey: false, altKey: false, metaKey: false })).toBeNull();
    });
});
