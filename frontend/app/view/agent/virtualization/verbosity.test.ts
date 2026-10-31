// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// Docs: docs/specs/SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29.md §4.8.

import { describe, expect, it } from "vitest";
import type { DocumentNode } from "../types";
import { rowDisclosure } from "./disclosure";
import {
    deriveDisplayNodes,
    hiddenRunId,
    hiddenRunLabel,
    nextPreset,
    parseView,
    pinStillHolds,
    presetOf,
    resolveView,
    ruleOf,
    SWITCH_KEYS,
    SWITCH_VALUES,
    VIEW_PRESETS,
    type AgentView,
} from "./verbosity";

const NONE = { pinnedNodes: new Set<string>(), heldOpenNodes: new Set<string>() };

// One node of every kind the pane stores, plus the variants that change which rule applies.
const tool = (id: string, status: "running" | "success" | "failed" | "pending_approval" | "awaiting_answer" = "success"): DocumentNode =>
    ({ type: "tool", id, tool: "Bash", params: {}, status, collapsed: true, summary: "", timestamp: 1 }) as unknown as DocumentNode;
const thought = (id: string): DocumentNode => ({ type: "markdown", id, content: "hmm", metadata: { thinking: true } });
const reply = (id: string): DocumentNode => ({ type: "markdown", id, content: "Done." });
const jekt = (id: string, tier = "coord", trust = "host-verified"): DocumentNode =>
    ({ type: "jekt_message", id, from: "a", to: "b", message: "m", raw: "", tier, deliveryTier: "host", trust, msgId: id, priority: "normal", direction: "incoming", timestamp: 1 }) as unknown as DocumentNode;
const note = (id: string, text = "Upgraded"): DocumentNode => ({ type: "system_note", id, text });
const user = (id: string, isStartup = false): DocumentNode => ({ type: "user_message", id, message: "hi", isStartup });
const startup = (id: string): DocumentNode => user(id, true);
const error = (id: string): DocumentNode => ({ type: "agent_error", id }) as unknown as DocumentNode;

const ALL_KINDS: DocumentNode[] = [
    thought("k1"),
    reply("k2"),
    tool("k3"),
    { type: "agent_message", id: "k4", message: "x" } as unknown as DocumentNode,
    jekt("k5"),
    note("k6"),
    { type: "ambient_narration", id: "k7", text: "x" } as unknown as DocumentNode,
    user("k8"),
    startup("k9"),
    { type: "shell", id: "k10" } as unknown as DocumentNode,
    { type: "context_delivery", id: "k11", items: [] } as unknown as DocumentNode,
    { type: "compaction_started", id: "k12" } as unknown as DocumentNode,
    { type: "memory_reinjection", id: "k13" } as unknown as DocumentNode,
    { type: "cli_notice", id: "k14", kind: "agent_ready", provider: "claude" } as unknown as DocumentNode,
    error("k15"),
    { type: "context_compacted", id: "k16" } as unknown as DocumentNode,
    { type: "session_outcome", id: "k17" } as unknown as DocumentNode,
    { type: "day_divider", id: "k18" } as unknown as DocumentNode,
    { type: "history_link", id: "k19" } as unknown as DocumentNode,
    { type: "resume_preflight", id: "k20" } as unknown as DocumentNode,
];

describe("presets and parsing", () => {
    it("derives the preset from the five values, and Custom when none match", () => {
        for (const name of ["full", "standard", "replies"] as const) expect(presetOf(VIEW_PRESETS[name])).toBe(name);
        expect(presetOf({ ...VIEW_PRESETS.standard, tools: "expanded" })).toBe("custom");
        // Setting the values back gives the preset back.
        expect(presetOf({ ...VIEW_PRESETS.standard, tools: "collapsed" })).toBe("standard");
    });

    it("sets each switch from a preset, and a single change reads Custom", () => {
        for (const key of SWITCH_KEYS) {
            for (const value of SWITCH_VALUES[key]) {
                const view = { ...VIEW_PRESETS.full, [key]: value } as AgentView;
                const matches = presetOf(view) !== "custom";
                expect(matches).toBe(value === VIEW_PRESETS.full[key]);
            }
        }
    });

    it("reads a preset name or an object, fills what is missing with Full, and refuses the rest", () => {
        expect(parseView("Standard")).toEqual(VIEW_PRESETS.standard);
        expect(parseView({ tools: "collapsed" })).toEqual({ ...VIEW_PRESETS.full, tools: "collapsed" });
        expect(parseView({ tools: "nonsense", notes: "hidden" })).toEqual({ ...VIEW_PRESETS.full, notes: "hidden" });
        expect(parseView("loud")).toBeUndefined();
        expect(parseView(null)).toBeUndefined();
        expect(parseView([1])).toBeUndefined();
    });

    it("resolves a pane that follows the default, one with its own view, and an unset default", () => {
        expect(resolveView(undefined, undefined)).toEqual({ view: VIEW_PRESETS.full, source: "default" });
        expect(resolveView("standard", undefined)).toEqual({ view: VIEW_PRESETS.standard, source: "default" });
        expect(resolveView("standard", "replies")).toEqual({ view: VIEW_PRESETS.replies, source: "pane" });
        // No per-key merging: the pane's own view is complete.
        expect(resolveView({ tools: "hidden" }, { thinking: "collapsed" }).view).toEqual({ ...VIEW_PRESETS.full, thinking: "collapsed" });
        // An unreadable pane value follows the default.
        expect(resolveView("standard", "garbage").source).toBe("default");
    });

    it("cycles Full, Standard, Replies, and a Custom view goes to Full", () => {
        expect(nextPreset(VIEW_PRESETS.full)).toBe("standard");
        expect(nextPreset(VIEW_PRESETS.standard)).toBe("replies");
        expect(nextPreset(VIEW_PRESETS.replies)).toBe("full");
        expect(nextPreset({ ...VIEW_PRESETS.standard, jekts: "auto" })).toBe("full");
    });
});

describe("which switch governs a row", () => {
    it("has a rule for every kind of row (a new kind is a compile error until it has one)", () => {
        for (const node of ALL_KINDS) expect(() => ruleOf(node as Exclude<DocumentNode, { type: "hidden_run" }>)).not.toThrow();
        const governed = ALL_KINDS.filter((n) => ruleOf(n as never).key !== null).map((n) => n.id);
        expect(governed).toEqual(["k1", "k3", "k4", "k5", "k6", "k7", "k9", "k10", "k11", "k12", "k13", "k14"]);
    });

    it("keeps the replies, your messages and the things that explain them outside every switch", () => {
        for (const id of ["k2", "k8", "k15", "k16", "k17", "k18", "k19", "k20"]) {
            expect(ruleOf(ALL_KINDS.find((n) => n.id === id) as never).key).toBeNull();
        }
    });

    it("treats a problem notice as unswitched, and information as setup", () => {
        const notice = (kind: string, extra: object) => ({ type: "cli_notice", id: "n", kind, provider: "claude", ...extra }) as never;
        expect(ruleOf(notice("install", { state: "failed" })).key).toBeNull();
        expect(ruleOf(notice("install", { state: "installed" })).key).toBe("context");
        expect(ruleOf(notice("compaction_failed", {})).key).toBeNull();
        expect(ruleOf(notice("agent_ready", {})).key).toBe("context");
    });
});

describe("never hide", () => {
    const awaiting: DocumentNode[] = [
        tool("w1", "pending_approval"),
        tool("w2", "awaiting_answer"),
        jekt("w3", "sensitive"),
        jekt("w4", "info", "unverified"),
        jekt("w4b", "info", "self-declared"),
        jekt("w4c", "coord", "network-claimed"),
        { type: "cli_notice", id: "w4d", kind: "container_limit", limitState: "oom", provider: "claude" } as unknown as DocumentNode,
        { type: "cli_notice", id: "w4e", kind: "container_limit", limitState: "failed", provider: "claude" } as unknown as DocumentNode,
        { type: "cli_notice", id: "w4f", kind: "container_caches", cacheBytes: 25 * 1024 ** 3, provider: "claude" } as unknown as DocumentNode,
        note("w5", "Cloud message expired undelivered"),
        user("w6"),
        error("w7"),
        { type: "context_compacted", id: "w8" } as unknown as DocumentNode,
        { type: "session_outcome", id: "w9" } as unknown as DocumentNode,
        reply("w10"),
    ];

    function combos(): AgentView[] {
        const out: AgentView[] = [];
        for (const thinking of SWITCH_VALUES.thinking)
            for (const tools of SWITCH_VALUES.tools)
                for (const jekts of SWITCH_VALUES.jekts)
                    for (const notes of SWITCH_VALUES.notes)
                        for (const context of SWITCH_VALUES.context) out.push({ thinking, tools, jekts, notes, context });
        return out;
    }

    it("shows every row on the list in every combination of the five switches", () => {
        const all = combos();
        expect(all).toHaveLength(4 * 4 * 4 * 2 * 2);
        for (const view of all) {
            const shown = deriveDisplayNodes([...awaiting, ...ALL_KINDS], view, NONE).map((n) => n.id);
            for (const node of awaiting) expect(shown, JSON.stringify(view)).toContain(node.id);
        }
    });

    it("keeps a pinned row, whatever the switches say", () => {
        const pinned = { ...NONE, pinnedNodes: new Set(["k3"]) };
        const shown = deriveDisplayNodes([reply("a"), tool("k3"), reply("b")], VIEW_PRESETS.replies, pinned).map((n) => n.id);
        expect(shown).toEqual(["a", "k3", "b"]);
    });
});

describe("the display list", () => {
    it("is the same list, not a copy, in Full", () => {
        const nodes = [thought("a"), tool("b"), jekt("c")];
        expect(deriveDisplayNodes(nodes, VIEW_PRESETS.full, NONE)).toBe(nodes);
    });

    it("hides a run of rows as one line with counts by kind, and keeps the rest", () => {
        const nodes = [user("u"), thought("t1"), tool("c1"), tool("c2", "failed"), thought("t2"), reply("r"), tool("c3")];
        const list = deriveDisplayNodes(nodes, VIEW_PRESETS.replies, NONE);
        expect(list.map((n) => n.id)).toEqual(["u", hiddenRunId("t1"), "r", hiddenRunId("c3")]);
        const run = list[1] as Extract<DocumentNode, { type: "hidden_run" }>;
        expect(run.counts).toEqual([
            { kind: "thinking", count: 2 },
            { kind: "tool", count: 2 },
        ]);
        expect(run.failed).toBe(1);
        expect(run.hiddenIds).toEqual(["t1", "c1", "c2", "t2"]);
        expect(hiddenRunLabel(run.counts)).toBe("2 thoughts, 2 tool calls");
        expect(hiddenRunLabel([{ kind: "jekt", count: 1 }])).toBe("1 jekt");
    });

    it("keeps a run's id and counts steady when rows are appended after it", () => {
        const base = [reply("r"), tool("c1"), tool("c2")];
        const a = deriveDisplayNodes(base, VIEW_PRESETS.replies, NONE)[1];
        const b = deriveDisplayNodes([...base, tool("c3")], VIEW_PRESETS.replies, NONE)[1];
        expect(a.id).toBe(b.id);
        expect((b as { hiddenIds: string[] }).hiddenIds).toHaveLength(3);
    });

    it("opens a run in place: its rows follow it in their full form", () => {
        const nodes = [reply("r"), tool("c1"), thought("t1")];
        const open = { ...NONE, pinnedNodes: new Set([hiddenRunId("c1")]) };
        const list = deriveDisplayNodes(nodes, VIEW_PRESETS.replies, open);
        expect(list.map((n) => n.id)).toEqual(["r", hiddenRunId("c1"), "c1", "t1"]);
        expect((list[1] as { open: boolean }).open).toBe(true);
        expect(list[2]).toBe(nodes[1]);
    });

    it("gives Collapsed and Expanded rows a viewMode on a copy, and the same copy each time", () => {
        const nodes = [tool("c1"), jekt("j1"), reply("r")];
        const a = deriveDisplayNodes(nodes, VIEW_PRESETS.standard, NONE);
        const b = deriveDisplayNodes(nodes, VIEW_PRESETS.standard, NONE);
        expect(a.map((n) => (n as { viewMode?: string }).viewMode)).toEqual(["collapsed", "collapsed", undefined]);
        expect(nodes[0]).not.toHaveProperty("viewMode");
        expect(a[0]).toBe(b[0]);
        expect(a[2]).toBe(nodes[2]);
        const expanded = deriveDisplayNodes(nodes, { ...VIEW_PRESETS.full, tools: "expanded" }, NONE);
        expect((expanded[0] as { viewMode?: string }).viewMode).toBe("expanded");
    });

    it("hides setup in Standard and keeps notes, and hides notes in Replies", () => {
        const nodes = [startup("s"), note("n"), reply("r")];
        expect(deriveDisplayNodes(nodes, VIEW_PRESETS.standard, NONE).map((n) => n.id)).toEqual([hiddenRunId("s"), "n", "r"]);
        expect(deriveDisplayNodes([note("n"), reply("r")], VIEW_PRESETS.replies, NONE).map((n) => n.id)).toEqual([hiddenRunId("n"), "r"]);
    });

    it("keeps a tool that was held open as a fold when tools are hidden, until it is released", () => {
        const held = { ...NONE, heldOpenNodes: new Set(["c1"]) };
        const list = deriveDisplayNodes([reply("r"), tool("c1"), tool("c2")], VIEW_PRESETS.replies, held);
        expect(list.map((n) => n.id)).toEqual(["r", "c1", hiddenRunId("c2")]);
        expect((list[1] as { viewMode?: string }).viewMode).toBe("collapsed");
        expect(deriveDisplayNodes([reply("r"), tool("c1")], VIEW_PRESETS.replies, NONE).map((n) => n.id)).toEqual(["r", hiddenRunId("c1")]);
    });

    it("lets the hidden-run line be a fixed row", () => {
        const run = deriveDisplayNodes([reply("r"), tool("c1")], VIEW_PRESETS.replies, NONE)[1];
        expect(rowDisclosure(run, {})).toMatchObject({ open: true, toggle: null });
    });
});

describe("rowDisclosure with a view mode", () => {
    const flags = { pinned: false, collapsed: false, held: false };

    it("Collapsed is closed even while running or held open, and a pin opens it", () => {
        const running = { ...(tool("c", "running") as object), viewMode: "collapsed" } as DocumentNode;
        expect(rowDisclosure(running, flags).open).toBe(false);
        expect(rowDisclosure(running, { ...flags, held: true }).open).toBe(false);
        expect(rowDisclosure(running, { ...flags, pinned: true })).toMatchObject({ open: true, via: "pin", toggle: "pin" });
    });

    it("Expanded is open until the user folds it", () => {
        const done = { ...(tool("c")), viewMode: "expanded" } as DocumentNode;
        expect(rowDisclosure(done, flags)).toMatchObject({ open: true, toggle: "collapse" });
        expect(rowDisclosure(done, { ...flags, collapsed: true }).open).toBe(false);
    });

    it("leaves a row with no mode to its own rule", () => {
        expect(rowDisclosure(tool("c", "running"), flags).open).toBe(true);
        expect(rowDisclosure(tool("c"), flags).open).toBe(false);
    });

    it("lets Collapsed fold a thought, which Full leaves open", () => {
        expect(rowDisclosure(thought("t"), flags).open).toBe(true);
        expect(rowDisclosure({ ...(thought("t")), viewMode: "collapsed" } as DocumentNode, flags).open).toBe(false);
    });
});

describe("an opened hidden run's pin", () => {
    it("holds while its first hidden row is in the document, and ends with it", () => {
        const present = new Set(["t1", "t2"]);
        expect(pinStillHolds(hiddenRunId("t1"), present)).toBe(true);
        expect(pinStillHolds(hiddenRunId("gone"), present)).toBe(false);
    });

    it("holds an ordinary pin the way the prune always did", () => {
        expect(pinStillHolds("t2", new Set(["t2"]))).toBe(true);
        expect(pinStillHolds("t3", new Set(["t2"]))).toBe(false);
    });
});
