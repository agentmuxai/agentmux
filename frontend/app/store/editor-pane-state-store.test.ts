// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Unit tests for slice #10 editor-pane-state (Phase 1A).
 *
 * Pure reducer / slot store tests — no DOM, no CEF context, no view
 * wiring. The 17 numbered behaviours from the Phase 1A spec each map
 * to at least one `it(...)` below; the suite name calls out which
 * invariant the test pins. Invariant 16 (active id always points at
 * a tab or is null) is checked by `assertActiveInvariant` at the end
 * of every test that touches the tab list.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import {
    canonicalizePath,
    dispatch,
    dispatchIfRegistered,
    type EditorPaneEvent,
    type EditorPaneState,
    MAX_RECENTLY_CLOSED,
    registerEditorPane,
    resetAllSlots,
    setEventSink,
    snapshot,
    unregisterEditorPane,
    update,
    initialState,
} from "./editor-pane-state-store";

function assertActiveInvariant(state: EditorPaneState): void {
    if (state.tabs.length === 0) {
        expect(state.activeTabId).toBeNull();
        return;
    }
    expect(state.activeTabId).not.toBeNull();
    expect(state.tabs.find((t) => t.id === state.activeTabId)).toBeTruthy();
}

function eventTypes(events: EditorPaneEvent[]): string[] {
    return events.map((e) => e.type);
}

describe("editor-pane-state-store (slice #10, Phase 1A)", () => {
    afterEach(() => {
        resetAllSlots();
    });

    // ─── slot lifecycle ──────────────────────────────────────────────

    it("dispatch on unregistered blockId throws (no silent drops)", () => {
        expect(() =>
            dispatch("nope", { type: "OpenFile", path: "C:/x.ts" }),
        ).toThrowError(/unregistered pane/);
    });

    it("dispatchIfRegistered on unregistered blockId returns [] silently", () => {
        const events = dispatchIfRegistered("ghost", {
            type: "OpenFile",
            path: "C:/x.ts",
        });
        expect(events).toEqual([]);
    });

    it("registerEditorPane is idempotent — re-registering keeps state", () => {
        registerEditorPane("blk-1");
        dispatch("blk-1", { type: "OpenFile", path: "C:/x.ts" });
        registerEditorPane("blk-1"); // no-op
        expect(snapshot("blk-1")?.tabs.length).toBe(1);
    });

    it("unregisterEditorPane removes the slot", () => {
        registerEditorPane("blk-1");
        unregisterEditorPane("blk-1");
        expect(snapshot("blk-1")).toBeNull();
        expect(() =>
            dispatch("blk-1", { type: "OpenFile", path: "C:/x.ts" }),
        ).toThrowError(/unregistered pane/);
    });

    it("multi-pane: dispatches don't cross blockIds", () => {
        registerEditorPane("blk-a");
        registerEditorPane("blk-b");
        dispatch("blk-a", { type: "OpenFile", path: "C:/a.ts" });
        dispatch("blk-b", { type: "OpenFile", path: "C:/b.ts" });
        expect(snapshot("blk-a")?.tabs[0].filePath).toBe("c:/a.ts");
        expect(snapshot("blk-b")?.tabs[0].filePath).toBe("c:/b.ts");
    });

    it("emits events through the configured sink (whole batch)", () => {
        const sink = vi.fn<(events: EditorPaneEvent[]) => void>();
        setEventSink(sink);
        registerEditorPane("blk-1");
        dispatch("blk-1", { type: "OpenFile", path: "C:/x.ts" });
        expect(sink).toHaveBeenCalledTimes(1);
        const batch = sink.mock.calls[0][0];
        expect(batch[0].type).toBe("TabOpened");
    });

    it("snapshot returns null for unknown blockId", () => {
        expect(snapshot("nope")).toBeNull();
    });

    // ─── invariant 1: OpenFile with new path → appends + activates ───

    it("OpenFile (new path) appends a tab and activates it", () => {
        const s0 = initialState();
        const { state, events } = update(s0, {
            type: "OpenFile",
            path: "C:/repo/index.ts",
        });
        expect(state.tabs.length).toBe(1);
        expect(state.tabs[0].filePath).toBe("c:/repo/index.ts");
        expect(state.activeTabId).toBe(state.tabs[0].id);
        expect(events.length).toBe(1);
        expect(events[0].type).toBe("TabOpened");
        if (events[0].type === "TabOpened") {
            expect(events[0].atIndex).toBe(0);
            expect(events[0].filePath).toBe("c:/repo/index.ts");
        }
        assertActiveInvariant(state);
    });

    // ─── invariant 2: OpenFile (existing, canonicalized) → activate ──

    it("OpenFile (existing path, mixed slashes) activates existing tab, no append", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/repo/a.ts" }).state;
        const firstId = s.tabs[0].id;
        // switch to something else first so we observe TabActivated
        s = update(s, { type: "OpenFile", path: "C:/repo/b.ts" }).state;
        expect(s.tabs.length).toBe(2);
        expect(s.activeTabId).toBe(s.tabs[1].id);

        const r = update(s, { type: "OpenFile", path: "C:\\repo\\a.ts" });
        expect(r.state.tabs.length).toBe(2); // not appended
        expect(r.state.activeTabId).toBe(firstId);
        expect(eventTypes(r.events)).toEqual(["TabActivated"]);
        assertActiveInvariant(r.state);
    });

    // ─── invariant 3: CloseTab clean → drops + neighbour-activate ────

    it("CloseTab on clean tab drops it, activates right-neighbour, records recentlyClosed", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        s = update(s, { type: "OpenFile", path: "C:/b.ts" }).state;
        s = update(s, { type: "OpenFile", path: "C:/c.ts" }).state;
        const [a, b, c] = s.tabs;
        // activate the middle tab so its closure picks a right
        // neighbour rather than the trivial last-tab case.
        s = update(s, { type: "SwitchTab", tabId: b.id }).state;

        const r = update(s, { type: "CloseTab", tabId: b.id });
        expect(r.state.tabs.map((t) => t.id)).toEqual([a.id, c.id]);
        expect(r.state.activeTabId).toBe(c.id);
        expect(r.state.recentlyClosed.map((e) => e.filePath)).toEqual([
            "c:/b.ts",
        ]);
        expect(eventTypes(r.events)).toEqual(["TabClosed", "TabActivated"]);
        assertActiveInvariant(r.state);
    });

    it("CloseTab on rightmost active tab falls back to left neighbour", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        s = update(s, { type: "OpenFile", path: "C:/b.ts" }).state;
        const [a, b] = s.tabs;
        expect(s.activeTabId).toBe(b.id);
        const r = update(s, { type: "CloseTab", tabId: b.id });
        expect(r.state.activeTabId).toBe(a.id);
        assertActiveInvariant(r.state);
    });

    it("CloseTab on inactive tab does not change activeTabId", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        s = update(s, { type: "OpenFile", path: "C:/b.ts" }).state;
        const [a, b] = s.tabs;
        // active is b; close a (inactive)
        const r = update(s, { type: "CloseTab", tabId: a.id });
        expect(r.state.activeTabId).toBe(b.id);
        // no TabActivated since the active tab didn't move
        expect(eventTypes(r.events)).toEqual(["TabClosed"]);
        assertActiveInvariant(r.state);
    });

    // ─── invariant 4: CloseTab dirty + !force → confirm event ────────

    it("CloseTab on dirty tab without force returns RequestDirtyConfirm, no state change", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        const [a] = s.tabs;
        s = update(s, { type: "MarkDirty", tabId: a.id }).state;
        const before = s;
        const closeCmd = { type: "CloseTab" as const, tabId: a.id, force: false };
        const r = update(s, closeCmd);
        expect(r.state).toBe(before);
        expect(r.events.length).toBe(1);
        expect(r.events[0].type).toBe("RequestDirtyConfirm");
        if (r.events[0].type === "RequestDirtyConfirm") {
            expect(r.events[0].tabId).toBe(a.id);
            expect(r.events[0].originalCommand).toEqual(closeCmd);
        }
    });

    // ─── invariant 5: CloseTab dirty + force → drops same as clean ───

    it("CloseTab on dirty tab WITH force drops the tab", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        const [a] = s.tabs;
        s = update(s, { type: "MarkDirty", tabId: a.id }).state;
        const r = update(s, { type: "CloseTab", tabId: a.id, force: true });
        expect(r.state.tabs).toEqual([]);
        expect(r.state.activeTabId).toBeNull();
        expect(eventTypes(r.events)).toEqual(["TabClosed"]);
        assertActiveInvariant(r.state);
    });

    // ─── invariant 6: CloseTab on only tab → null active, empty list ─

    it("CloseTab on the only tab leaves the pane empty and active null", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        const [a] = s.tabs;
        const r = update(s, { type: "CloseTab", tabId: a.id });
        expect(r.state.tabs).toEqual([]);
        expect(r.state.activeTabId).toBeNull();
        assertActiveInvariant(r.state);
    });

    // ─── invariant 7: SwitchTab → unknown id → no-op ─────────────────

    it("SwitchTab to a non-existent tab id is a no-op, no events", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        const before = s;
        const r = update(s, { type: "SwitchTab", tabId: "no-such-tab" });
        expect(r.state).toBe(before);
        expect(r.events).toEqual([]);
        assertActiveInvariant(r.state);
    });

    it("SwitchTab to already-active id is a no-op", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        const [a] = s.tabs;
        const r = update(s, { type: "SwitchTab", tabId: a.id });
        expect(r.events).toEqual([]);
    });

    it("SwitchTab to a different existing tab updates activeTabId + emits TabActivated", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        s = update(s, { type: "OpenFile", path: "C:/b.ts" }).state;
        const [a, b] = s.tabs;
        expect(s.activeTabId).toBe(b.id);
        const r = update(s, { type: "SwitchTab", tabId: a.id });
        expect(r.state.activeTabId).toBe(a.id);
        expect(eventTypes(r.events)).toEqual(["TabActivated"]);
        assertActiveInvariant(r.state);
    });

    // ─── invariant 8: ReorderTab clamps toIndex ──────────────────────

    it("ReorderTab clamps toIndex to [0, tabs.length - 1]", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        s = update(s, { type: "OpenFile", path: "C:/b.ts" }).state;
        s = update(s, { type: "OpenFile", path: "C:/c.ts" }).state;
        const [a, b, c] = s.tabs;

        // negative clamps to 0 — moves a (idx 0) to 0 → no change
        let r = update(s, { type: "ReorderTab", tabId: a.id, toIndex: -5 });
        expect(r.state.tabs.map((t) => t.id)).toEqual([a.id, b.id, c.id]);

        // beyond last clamps to last (2) — moves a to the end
        r = update(s, { type: "ReorderTab", tabId: a.id, toIndex: 99 });
        expect(r.state.tabs.map((t) => t.id)).toEqual([b.id, c.id, a.id]);

        // mid-list move
        r = update(s, { type: "ReorderTab", tabId: c.id, toIndex: 0 });
        expect(r.state.tabs.map((t) => t.id)).toEqual([c.id, a.id, b.id]);
        assertActiveInvariant(r.state);
    });

    // ─── invariant 9: MarkDirty / ClearDirty emit only on transitions ─

    it("MarkDirty flips flag and emits TabDirtied, second call is a no-op", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        const [a] = s.tabs;
        let r = update(s, { type: "MarkDirty", tabId: a.id });
        expect(r.state.tabs[0].dirty).toBe(true);
        expect(eventTypes(r.events)).toEqual(["TabDirtied"]);
        const r2 = update(r.state, { type: "MarkDirty", tabId: a.id });
        expect(r2.events).toEqual([]);
        expect(r2.state).toBe(r.state);
    });

    it("ClearDirty flips flag and emits TabSaved, second call is a no-op", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        const [a] = s.tabs;
        s = update(s, { type: "MarkDirty", tabId: a.id }).state;
        let r = update(s, { type: "ClearDirty", tabId: a.id });
        expect(r.state.tabs[0].dirty).toBe(false);
        expect(eventTypes(r.events)).toEqual(["TabSaved"]);
        // ClearDirty on already-clean tab → no event
        const r2 = update(r.state, { type: "ClearDirty", tabId: a.id });
        expect(r2.events).toEqual([]);
    });

    // ─── invariant 10: TabContentLoaded sets loaded + hash, clears err ─

    it("TabContentLoaded sets contentLoaded, contentHash, clears loadError", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        const [a] = s.tabs;
        // seed a prior error so we can observe the clear
        s = update(s, {
            type: "TabContentLoadFailed",
            tabId: a.id,
            error: "boom",
        }).state;
        expect(s.tabs[0].loadError).toBe("boom");
        const r = update(s, {
            type: "TabContentLoaded",
            tabId: a.id,
            contentHash: "abc123",
        });
        expect(r.state.tabs[0].contentLoaded).toBe(true);
        expect(r.state.tabs[0].contentHash).toBe("abc123");
        expect(r.state.tabs[0].loadError).toBeNull();
    });

    // ─── invariant 11: TabContentLoadFailed sets error, keeps !loaded ─

    it("TabContentLoadFailed sets loadError and keeps contentLoaded false", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        const [a] = s.tabs;
        const r = update(s, {
            type: "TabContentLoadFailed",
            tabId: a.id,
            error: "ENOENT",
        });
        expect(r.state.tabs[0].loadError).toBe("ENOENT");
        expect(r.state.tabs[0].contentLoaded).toBe(false);
    });

    // ─── invariant 12: ReopenLastClosed on empty stack → no-op ──────

    it("ReopenLastClosed on empty stack is a no-op", () => {
        const s = initialState();
        const r = update(s, { type: "ReopenLastClosed" });
        expect(r.state).toBe(s);
        expect(r.events).toEqual([]);
    });

    // ─── invariant 13: ReopenLastClosed pops + dispatches OpenFile ───

    it("ReopenLastClosed pops the stack and re-opens the file", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        s = update(s, { type: "OpenFile", path: "C:/b.ts" }).state;
        const [a, b] = s.tabs;
        s = update(s, { type: "CloseTab", tabId: a.id }).state;
        // a is in recentlyClosed; b is the active sole tab
        expect(s.recentlyClosed.map((e) => e.filePath)).toEqual(["c:/a.ts"]);

        const r = update(s, { type: "ReopenLastClosed" });
        // a is re-opened (new tab id, same path), b stays
        expect(r.state.tabs.length).toBe(2);
        expect(r.state.tabs.map((t) => t.filePath).sort()).toEqual([
            "c:/a.ts",
            "c:/b.ts",
        ]);
        expect(r.state.recentlyClosed).toEqual([]);
        expect(eventTypes(r.events)).toEqual(["TabOpened"]);
        assertActiveInvariant(r.state);
    });

    // ─── invariant 14: HydrateFromMeta bulk-restores ────────────────

    it("HydrateFromMeta bulk-restores tabs + activeTabId, all contentLoaded=false, emits TabsRestored", () => {
        const s = initialState();
        const r = update(s, {
            type: "HydrateFromMeta",
            tabs: [
                { id: "t-1", filePath: "C:/repo/x.ts" },
                { id: "t-2", filePath: "C:/repo/y.ts" },
            ],
            activeTabId: "t-2",
        });
        expect(r.state.tabs.length).toBe(2);
        expect(r.state.tabs.every((t) => !t.contentLoaded)).toBe(true);
        expect(r.state.activeTabId).toBe("t-2");
        expect(r.state.tabs[0].filePath).toBe("c:/repo/x.ts"); // canonicalized
        expect(eventTypes(r.events)).toEqual(["TabsRestored"]);
        if (r.events[0].type === "TabsRestored") {
            expect(r.events[0].fromDefaults).toBe(false);
            expect(r.events[0].tabIds).toEqual(["t-1", "t-2"]);
        }
        assertActiveInvariant(r.state);
    });

    it("HydrateFromDefaults emits TabsRestored with fromDefaults=true", () => {
        const s = initialState();
        const r = update(s, {
            type: "HydrateFromDefaults",
            tabs: [{ id: "t-1", filePath: "C:/x.ts" }],
            activeTabId: "t-1",
        });
        if (r.events[0].type === "TabsRestored") {
            expect(r.events[0].fromDefaults).toBe(true);
        }
    });

    it("HydrateFromMeta with stale activeTabId falls back to first tab", () => {
        const r = update(initialState(), {
            type: "HydrateFromMeta",
            tabs: [{ id: "t-1", filePath: "C:/a.ts" }],
            activeTabId: "does-not-exist",
        });
        expect(r.state.activeTabId).toBe("t-1");
        assertActiveInvariant(r.state);
    });

    it("HydrateFromMeta with empty tabs → empty pane, active null", () => {
        const r = update(initialState(), {
            type: "HydrateFromMeta",
            tabs: [],
            activeTabId: null,
        });
        expect(r.state.tabs).toEqual([]);
        expect(r.state.activeTabId).toBeNull();
        assertActiveInvariant(r.state);
    });

    // ─── invariant 15: recentlyClosed capped at MAX_RECENTLY_CLOSED ──

    it("recentlyClosed capped at 10 — oldest evicted on overflow", () => {
        let s = initialState();
        // open + close 12 distinct tabs in sequence
        for (let i = 0; i < 12; i++) {
            s = update(s, { type: "OpenFile", path: `C:/file-${i}.ts` }).state;
            const id = s.tabs[s.tabs.length - 1].id;
            s = update(s, { type: "CloseTab", tabId: id }).state;
        }
        expect(s.recentlyClosed.length).toBe(MAX_RECENTLY_CLOSED);
        // the two oldest entries (file-0, file-1) were evicted
        const paths = s.recentlyClosed.map((e) => e.filePath);
        expect(paths[0]).toBe("c:/file-2.ts");
        expect(paths[paths.length - 1]).toBe("c:/file-11.ts");
    });

    // ─── invariant 17: tab ids are unique within a pane ─────────────

    it("Tab ids are unique within a pane (no dup-by-path)", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        s = update(s, { type: "OpenFile", path: "C:/b.ts" }).state;
        s = update(s, { type: "OpenFile", path: "C:/c.ts" }).state;
        // open an existing path — must NOT mint a new id
        s = update(s, { type: "OpenFile", path: "C:/a.ts" }).state;
        const ids = s.tabs.map((t) => t.id);
        expect(new Set(ids).size).toBe(ids.length);
        expect(s.tabs.length).toBe(3);
    });

    // ─── extra coverage: RenameFile + canonicalize + slot dispatch ──

    it("RenameFile updates a matching tab's filePath, no-op on no match", () => {
        let s = initialState();
        s = update(s, { type: "OpenFile", path: "C:/old.ts" }).state;
        const [t] = s.tabs;
        const r = update(s, {
            type: "RenameFile",
            oldPath: "C:/old.ts",
            newPath: "C:/new.ts",
        });
        expect(r.state.tabs[0].id).toBe(t.id);
        expect(r.state.tabs[0].filePath).toBe("c:/new.ts");

        const r2 = update(r.state, {
            type: "RenameFile",
            oldPath: "C:/missing.ts",
            newPath: "C:/x.ts",
        });
        expect(r2.state).toBe(r.state);
    });

    it("canonicalizePath normalizes slashes, drive case, doubled slashes", () => {
        expect(canonicalizePath("C:\\Repo\\Foo.ts")).toBe("c:/Repo/Foo.ts");
        expect(canonicalizePath("C:/a//b/c")).toBe("c:/a/b/c");
        expect(canonicalizePath("C:/a/")).toBe("c:/a");
        expect(canonicalizePath("")).toBe("");
    });

    // Regression for a live-repro'd bug (2026-08-22): Rust's
    // Path::canonicalize() unconditionally prepends `\\?\` on Windows.
    // EditorFileWatcher's editor:file_changed WPS event carries that
    // prefixed path; without stripping it here, it can never compare
    // equal to a tab's own (never-prefixed) filePath, so live-reload
    // silently never fires on Windows at all.
    it("canonicalizePath strips Windows' \\\\?\\ extended-length prefix so it matches the un-prefixed form", () => {
        const prefixed = canonicalizePath("\\\\?\\C:\\Users\\asafe\\AppData\\Local\\Temp\\probe.md");
        const unprefixed = canonicalizePath("C:\\Users\\asafe\\AppData\\Local\\Temp\\probe.md");
        expect(prefixed).toBe(unprefixed);
        expect(prefixed).toBe("c:/Users/asafe/AppData/Local/Temp/probe.md");
    });

    it("canonicalizePath strips the \\\\?\\UNC\\ variant for network shares", () => {
        const prefixed = canonicalizePath("\\\\?\\UNC\\server\\share\\file.md");
        // The function's existing (pre-existing, unrelated to this fix)
        // doubled-slash collapse already reduces a plain UNC path's
        // leading "\\\\" to a single "/" — this test only pins down that
        // the \\?\UNC\ prefix itself is stripped consistently with that
        // existing behavior, not a claim of correct UNC round-tripping.
        expect(prefixed.startsWith("\\\\?\\")).toBe(false);
        expect(prefixed).toBe("/server/share/file.md");
    });

    it("slot dispatch path: OpenFile via dispatch updates the snapshot", () => {
        registerEditorPane("blk-x");
        const events = dispatch("blk-x", { type: "OpenFile", path: "C:/foo.ts" });
        expect(events.length).toBe(1);
        expect(events[0].type).toBe("TabOpened");
        const snap = snapshot("blk-x")!;
        expect(snap.tabs.length).toBe(1);
        expect(snap.activeTabId).toBe(snap.tabs[0].id);
        assertActiveInvariant(snap);
    });

    it("setEventSink(null) detaches the sink", () => {
        const sink = vi.fn<(events: EditorPaneEvent[]) => void>();
        setEventSink(sink);
        registerEditorPane("blk-1");
        dispatch("blk-1", { type: "OpenFile", path: "C:/a.ts" });
        expect(sink).toHaveBeenCalledTimes(1);
        setEventSink(null);
        dispatch("blk-1", { type: "OpenFile", path: "C:/b.ts" });
        expect(sink).toHaveBeenCalledTimes(1); // not called again
    });
});
