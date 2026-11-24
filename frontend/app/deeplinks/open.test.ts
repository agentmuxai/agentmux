// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import type { PanePlan } from "./destination-meta";
import { DEEPLINKS, deepLinkView } from "./registry";
import {
    AGENT_PANES_PER_TAB,
    Executor,
    PERSON_OPENS_PER_SECOND,
    replaceRule,
    TYPING_MAX_WAIT_MS,
    type ExecutorDeps,
    type PaneFacts,
} from "./open";
import type { OpenSource } from "./open-types";

const PERSON: OpenSource = { kind: "person", via: "click" };
const AGENT: OpenSource = { kind: "agent", agentId: "sorrel", pane: "conv", requestId: "r1" };

/** A window of panes held in memory: `panes` by block id, each with its view, meta and tab. */
function fakeWindow(start: Record<string, { view: string; tab?: string; facts?: Partial<PaneFacts> }> = {}) {
    let clock = 1_000_000;
    let next = 1;
    const panes = new Map<string, { view: string; meta: Record<string, unknown>; tab: string; facts: Partial<PaneFacts> }>();
    for (const [id, p] of Object.entries(start)) panes.set(id, { view: p.view, meta: { view: p.view }, tab: p.tab ?? "t1", facts: p.facts ?? {} });
    panes.set("conv", { view: "agent", meta: { view: "agent" }, tab: "t1", facts: {} });
    const make = (plan: PanePlan, tab = "t1") => {
        const id = `b${next++}`;
        panes.set(id, { view: plan.view, meta: { ...plan.meta }, tab, facts: {} });
        return id;
    };
    const calls = { person: 0, withoutFocus: 0, stack: 0, setMeta: [] as [string, Record<string, unknown>][], announced: [] as string[], located: [] as string[] };
    let input = { lastKeyAt: Number.NEGATIVE_INFINITY, recording: false, composing: false };
    const deps: ExecutorDeps = {
        now: () => clock,
        sleep: async (ms) => {
            clock += ms;
        },
        translate: (k) => ({ "deeplinks.views.settings": "Settings", "settings.sections.recording": "Recording" })[k] ?? k,
        async openForPerson(plan, how) {
            calls.person++;
            if (!how.newPane && !how.floating) {
                const found = await deps.findPane(plan);
                if (found) {
                    await deps.setMeta(found, plan.focusMeta);
                    return found;
                }
            }
            return make(plan);
        },
        async findPane(plan, tabId = "t1") {
            for (const [id, p] of panes) if (p.view === plan.view && p.tab === tabId) return id;
            return null;
        },
        async setMeta(blockId, meta) {
            calls.setMeta.push([blockId, meta]);
            const p = panes.get(blockId);
            if (p) p.meta = { ...p.meta, ...meta };
        },
        async showWithoutFocus() {},
        async openWithoutFocus(plan, at) {
            calls.withoutFocus++;
            return make(plan, at.tabId ?? "t1");
        },
        async stackOnto(plan) {
            calls.stack++;
            return make(plan);
        },
        paneFacts(blockId) {
            const p = panes.get(blockId);
            return p ? { view: p.view, ...p.facts } : undefined;
        },
        tabOf: (blockId) => panes.get(blockId)?.tab,
        tabExists: (tabId) => tabId === "t1" || tabId === "t2",
        settingExists: async (id) => id === "recording.engine",
        vaultEntryExists: async (id) => id === "e_ab12cd34",
        locate: (blockId) => calls.located.push(blockId),
        announce: (text) => calls.announced.push(text),
        personInput: () => input,
        openExternal: vi.fn(async () => {}),
        openBrowserPane: vi.fn(async () => "web1"),
    };
    return {
        deps,
        panes,
        calls,
        tick: (ms: number) => (clock += ms),
        setInput: (i: typeof input) => (input = i),
        now: () => clock,
    };
}

describe("openDestination: outcomes", () => {
    it("opens a new pane, then reuses it for the same destination and reveals again (R4)", async () => {
        const w = fakeWindow();
        const ex = new Executor(w.deps);
        const first = await ex.open("agentmux://settings/recording", { source: PERSON });
        expect(first.outcome).toBe("opened");
        expect(first.label).toBe("Settings › Recording");
        w.tick(1000);
        const again = await ex.open("agentmux://settings/recording", { source: PERSON });
        expect(again.outcome).toBe("reused");
        expect(again.blockId).toBe(first.blockId);
        expect(w.calls.setMeta.at(-1)).toEqual([first.blockId, { "settings:section": "recording" }]);
    });

    it("refuses a link the parser refuses, and a later build's view as not_yet", async () => {
        const ex = new Executor(fakeWindow().deps);
        expect((await ex.open("agentmux://nowhere", { source: PERSON })).reason?.code).toBe("invalid_link");
        expect((await ex.open("https://example.com", { source: PERSON })).reason?.code).toBe("invalid_link");
        expect((await ex.open("agentmux://statusbar/cpu", { source: PERSON })).reason?.code).toBe("not_yet");
        expect((await ex.open("agentmux://term/b1", { source: AGENT })).reason?.code).toBe("not_yet");
    });

    it("takes a request and a parsed link through the same reading as a string (one validation path)", async () => {
        const w = fakeWindow();
        const ex = new Executor(w.deps);
        const out = await ex.open({ view: "kanban", place: "task", values: { task: 12 } }, { source: PERSON });
        expect(out.outcome).toBe("opened");
        expect(w.panes.get(out.blockId!)?.meta["kanban:open"]).toBe("12");
        expect((await ex.open({ view: "kanban", place: "task", values: { task: -1 } }, { source: PERSON })).reason?.code).toBe("invalid_link");
        expect((await ex.open({ view: "settings", place: "nowhere" }, { source: PERSON })).reason?.code).toBe("invalid_link");
    });

    it("opens the view with place missing when the place is gone (§8)", async () => {
        const ex = new Executor(fakeWindow().deps);
        const gone = await ex.open("agentmux://settings/recording#recording.gone", { source: PERSON });
        expect(gone.outcome).toBe("opened");
        expect(gone.place).toBe("missing");
        const there = await ex.open("agentmux://settings/recording#recording.engine", { source: PERSON });
        expect(there.place).toBeUndefined();
        expect((await ex.open("agentmux://vault/entry/e_ab12cd34", { source: PERSON })).place).toBeUndefined();
        expect((await ex.open("agentmux://vault/entry/e_ffff0000", { source: PERSON })).place).toBe("missing");
    });
});

describe("agent opens", () => {
    it("open beside the conversation without focus, and are announced politely (G13)", async () => {
        const w = fakeWindow();
        const ex = new Executor(w.deps);
        const out = await ex.open("agentmux://settings/recording", { source: AGENT });
        expect(out.outcome).toBe("opened");
        expect(w.calls.person).toBe(0);
        expect(w.calls.withoutFocus).toBe(1);
        expect(w.calls.announced).toEqual(["Opened Settings › Recording beside this conversation"]);
        expect(w.calls.located).toEqual([out.blockId]);
    });

    it("are refused another window", async () => {
        const ex = new Executor(fakeWindow().deps);
        expect((await ex.open("agentmux://history", { source: AGENT, target: { kind: "window" } })).reason?.code).toBe("another_window");
    });

    it("need the named tab to exist", async () => {
        const w = fakeWindow();
        const ex = new Executor(w.deps);
        expect((await ex.open("agentmux://history", { source: AGENT, target: { kind: "tab", tabId: "gone" } })).reason?.code).toBe("not_open");
        const out = await ex.open("agentmux://history", { source: AGENT, target: { kind: "tab", tabId: "t2" } });
        expect(w.panes.get(out.blockId!)?.tab).toBe("t2");
    });

    it("stop at the caps of unused panes", async () => {
        const w = fakeWindow();
        const ex = new Executor(w.deps);
        const views = ["history", "atlas", "swarm", "drone", "toolchain", "tower", "help", "shortcuts", "sysinfo"];
        const outs = [];
        for (const v of views) outs.push(await ex.open(`agentmux://${v}`, { source: AGENT }));
        expect(outs.slice(0, AGENT_PANES_PER_TAB).every((o) => o.outcome === "opened")).toBe(true);
        expect(outs[AGENT_PANES_PER_TAB].reason?.code).toBe("too_many_panes");
        // One the person used no longer counts.
        w.panes.get(outs[0].blockId!)!.facts.usedByPerson = true;
        expect((await ex.open("agentmux://sysinfo", { source: AGENT })).outcome).toBe("opened");
    });

    it("wait while the person types, at most ten seconds", async () => {
        const w = fakeWindow();
        const ex = new Executor(w.deps);
        w.setInput({ lastKeyAt: w.now(), recording: false, composing: true });
        const start = w.now();
        const out = await ex.open("agentmux://history", { source: AGENT });
        expect(out.outcome).toBe("opened");
        expect(w.now() - start).toBeGreaterThanOrEqual(TYPING_MAX_WAIT_MS);
        expect(w.now() - start).toBeLessThan(TYPING_MAX_WAIT_MS + 1000);
    });

    it("reuse a named pane of the same view, and open beside one with work in it (R5)", async () => {
        const w = fakeWindow({ s1: { view: "settings" }, ed: { view: "editor", facts: { editorDirty: true } } });
        const ex = new Executor(w.deps);
        const reused = await ex.open("agentmux://settings/recording", { source: AGENT, target: { kind: "pane", blockId: "s1" } });
        expect(reused).toMatchObject({ outcome: "reused", blockId: "s1" });
        const beside = await ex.open("agentmux://history", { source: AGENT, target: { kind: "pane", blockId: "ed" } });
        expect(beside.outcome).toBe("opened");
        expect(beside.blockId).not.toBe("ed");
        expect(beside.reason?.code).toBe("would_replace");
        expect(w.panes.get("ed")?.view).toBe("editor");
    });
});

describe("R5: never replaced", () => {
    const view = "history";
    it.each<[string, PaneFacts]>([
        ["an agent pane with a running agent", { view: "agent", agentRunning: true, openedBy: "sorrel" }],
        ["a terminal with a process", { view: "term", termProcess: true, openedBy: "sorrel" }],
        ["an editor with unsaved changes", { view: "editor", editorDirty: true, openedBy: "sorrel" }],
        ["a browser pane", { view: "browser", openedBy: "sorrel" }],
        ["a pane the person has used", { view: "atlas", usedByPerson: true, openedBy: "sorrel" }],
        ["a pane another agent opened", { view: "atlas", openedBy: "other" }],
        ["a pane the person opened", { view: "atlas" }],
    ])("%s", (_name, facts) => {
        expect(replaceRule(facts, view, AGENT)).toBe("never");
    });

    it("a pane of the same view is reused, and one the agent opened and nobody used may be replaced", () => {
        expect(replaceRule({ view }, view, AGENT)).toBe("reuse");
        expect(replaceRule({ view: "atlas", openedBy: "sorrel" }, view, AGENT)).toBe("replace");
        expect(replaceRule({ view: "atlas", openedBy: "sorrel" }, view, PERSON)).toBe("never");
    });
});

describe("person opens", () => {
    it("are at most five a second (G11)", async () => {
        const ex = new Executor(fakeWindow().deps);
        const views = ["history", "atlas", "swarm", "drone", "toolchain", "tower"];
        const outs = [];
        for (const v of views) outs.push(await ex.open(`agentmux://${v}`, { source: PERSON }));
        expect(outs.slice(0, PERSON_OPENS_PER_SECOND).every((o) => o.outcome !== "refused")).toBe(true);
        expect(outs[PERSON_OPENS_PER_SECOND].reason?.code).toBe("rate_limited");
    });

    it("are one open per click: the same destination asked again while it runs is the same open (G10)", async () => {
        const w = fakeWindow();
        const ex = new Executor(w.deps);
        const [a, b] = await Promise.all([ex.open("agentmux://history", { source: PERSON }), ex.open("agentmux://history", { source: PERSON })]);
        expect(a).toBe(b);
        expect(w.calls.person).toBe(1);
    });

    it("never go through the agent path, and move nothing on their own (G13: focus is the window opener's)", async () => {
        const w = fakeWindow();
        const ex = new Executor(w.deps);
        await ex.open("agentmux://history", { source: PERSON });
        expect(w.calls.withoutFocus).toBe(0);
        expect(w.calls.announced).toEqual([]);
    });
});

describe("G1: opening a first-build destination changes nothing and runs nothing", () => {
    // Every first-build view at its plain form: the open writes pane meta, and nothing else the window does is called.
    const links = DEEPLINKS.views.filter((v) => v.build <= DEEPLINKS.build && !v.requiresPlace).map((v) => `agentmux://${v.id}`);
    it.each(links)("%s", async (link) => {
        const w = fakeWindow();
        const ex = new Executor(w.deps);
        const out = await ex.open(link, { source: AGENT });
        expect(out.outcome).not.toBe("refused");
        expect(deepLinkView(link.slice("agentmux://".length))?.effect).toBe("none");
        expect(w.deps.openExternal).not.toHaveBeenCalled();
        expect(w.deps.openBrowserPane).not.toHaveBeenCalled();
        // The only writes are to the opened pane's own meta.
        for (const [blockId] of w.calls.setMeta) expect(blockId).toBe(out.blockId);
    });
});

describe("wrappers", () => {
    it("openTask and openFile are one openDestination each", async () => {
        const w = fakeWindow();
        const ex = new Executor(w.deps);
        const ws = ex.wrappers();
        const task = await ws.openTask({ number: 12, board: "work" }, { source: PERSON });
        expect(w.panes.get(task.blockId!)?.meta["kanban:open"]).toBe("work#12");
        w.tick(1000);
        const file = await ws.openFile({ path: "C:/work/a.ts", line: 3 }, { source: PERSON });
        expect(w.panes.get(file.blockId!)?.meta).toMatchObject({ view: "editor", file: "C:/work/a.ts", "editor:line": 3 });
    });

    it("openUrl: the person's browser, or a browser pane when floating", async () => {
        const w = fakeWindow();
        const ws = new Executor(w.deps).wrappers();
        expect((await ws.openUrl("https://example.com", { source: PERSON })).outcome).toBe("opened");
        expect(w.deps.openExternal).toHaveBeenCalledWith("https://example.com");
        expect((await ws.openUrl("https://example.com", { source: PERSON, floating: true })).blockId).toBe("web1");
        expect((await ws.openUrl("file:///etc/passwd", { source: PERSON })).reason?.code).toBe("invalid_link");
    });

    it("openAgent and openTab are build 2", async () => {
        const ws = new Executor(fakeWindow().deps).wrappers();
        expect((await ws.openAgent({ id: "a", drawnByApp: true }, { source: PERSON })).reason?.code).toBe("not_yet");
        expect((await ws.openTab({ tabId: "t1" }, { source: PERSON })).reason?.code).toBe("not_yet");
    });

    it("canOpen answers without opening anything", () => {
        const w = fakeWindow();
        const ws = new Executor(w.deps).wrappers();
        expect(ws.canOpen("agentmux://settings/recording", PERSON)).toEqual({ ok: true, label: "Settings › Recording" });
        expect(ws.canOpen("agentmux://statusbar/cpu", PERSON)).toMatchObject({ ok: false, code: "not_yet" });
        expect(w.calls.person + w.calls.withoutFocus).toBe(0);
    });
});
