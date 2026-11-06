// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActivityRow, pinned as a whole ahead of splitting it into a state object and view parts: each scenario mounts the
 * real row with stand-ins for the clock, the pulse, the sigil, the tooltip, the buttons, the log preview and the
 * subagent transcript, and logs the row as an indented tree and, in order, every hook it builds and every callback
 * its children press. The logs are snapshots recorded on the unsplit component; the split must leave them byte for
 * byte. (The ActivityRow.*.test.tsx files assert what the person sees; this pins the markup, the wiring and the
 * order things are built in.) The stand-in builder is the one AgentBottomPanels.behavior.test.tsx uses.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { calls as T } from "./agent-bottom-panels.behavior.stub";

const h = vi.hoisted(() => ({ tick: null as any, entries: null as any }));
const log = (line: string): void => {
    T.push(line);
};

vi.mock("@/app/hook/useTick", () => ({
    useTick: (ms: number) => (log(`useTick ${ms}`), h.tick[0]),
}));
vi.mock("@/app/motion/solid", async () => {
    const { createEffect } = await import("solid-js");
    return {
        usePulseWhen: (el: () => HTMLElement | undefined, when: () => boolean) => {
            log("usePulseWhen");
            createEffect(() => log(`pulse when=${when()} on=${el()?.className}`));
        },
    };
});
vi.mock("./ActivitySigil", () => ({
    ActivitySigil: (p: { activity: { id: string }; glyph: string; tick: () => number }) => (
        <span data-stand="ActivitySigil" data-activity={p.activity.id} data-glyph={p.glyph} data-tick={p.tick()} />
    ),
}));
vi.mock("@/app/element/tooltip", () => ({
    Tooltip: (p: { content?: string; disable?: boolean; divClassName?: string; children?: JSX.Element }) => (
        <div data-stand="Tooltip" data-content={String(p.content)} data-disable={String(p.disable)} data-class={p.divClassName}>
            {p.children}
        </div>
    ),
}));
vi.mock("@/app/element/ui/Button", () => ({
    IconButton: (p: Record<string, any>) => (
        <span
            data-stand={`IconButton:${p.label}`}
            data-props={JSON.stringify({ icon: p.icon, tone: p.tone, density: p.density, class: p.class, tooltip: p.tooltip })}
            data-cb="onClick"
            onClick={(e) => {
                log(`IconButton:${p.label}.onClick`);
                const stop = e.stopPropagation.bind(e);
                e.stopPropagation = () => (log("stopPropagation"), stop());
                p.onClick(e);
            }}
        >
            {p.label}
        </span>
    ),
}));
vi.mock("./ChunkPreview", () => ({
    ChunkPreview: (p: { chunks: unknown[] }) => <div data-stand="ChunkPreview" data-chunks={JSON.stringify(p.chunks)} />,
}));
vi.mock("../../swarm/swarm-model", () => ({
    createDispatchDetail: (dispatchId: string, agentId: string) => {
        log(`createDispatchDetail ${dispatchId} ${agentId}`);
        return { entriesAtom: h.entries[0], dispose: () => log(`dispose ${dispatchId}`) };
    },
    subagentDisplayLabel: (m: { display_name?: string; agent_id: string }) => m.display_name ?? m.agent_id,
}));
vi.mock("../../../agent-model/tool-adapter", () => ({
    toolOutputChunks: (t: { fakeChunks?: unknown[] }) => (log("toolOutputChunks"), t.fakeChunks),
}));

import { ActivityRow } from "./ActivityRow";
import { EXIT_FLASH_MS, type PinnedActivity } from "../../../agent-model/activity-types";

const T0 = 1_800_000_000_000;
const SNAP = (name: string) => `./__snapshots__/ActivityRow.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type)$/;

/** The row as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
let setActivity: (a: PinnedActivity | undefined) => void;
let setExpanded: (v: boolean) => void;
let setLeaving: (v: boolean) => void;
let setEntries: (v: unknown[]) => void;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const advance = (ms: number) => {
    vi.advanceTimersByTime(ms);
    h.tick[1]((n: number) => n + 1);
};
const press = (selector: string, nth = 0) => fireEvent.click(host.querySelectorAll(selector)[nth]);

const activity = (over: Partial<PinnedActivity> = {}): PinnedActivity => ({
    id: "a1",
    kind: "shell",
    title: "npm run dev",
    status: "running",
    startedAt: Date.now() - 65_000,
    canStop: true,
    ...over,
});
const shell = (cmd: string, chunks: unknown[], open = false) => ({ cmd, log: { chunks, open } }) as never;
const sub = (over: Record<string, unknown> = {}) =>
    ({ dispatch_id: "d1", agent_id: "ag1", status: "active", event_count: 0, display_name: "Scout", ...over }) as never;

function mount(a: PinnedActivity | undefined, expanded = false, leaving = false) {
    const [act, setA] = createSignal<PinnedActivity | undefined>(a);
    const [exp, setE] = createSignal(expanded);
    const [lv, setL] = createSignal(leaving);
    setActivity = setA;
    setExpanded = setE;
    setLeaving = setL;
    host = render(() => (
        <ActivityRow
            activity={act}
            expanded={exp}
            leaving={lv}
            onToggle={() => log("onToggle")}
            onStop={() => log("onStop")}
            onDismiss={() => log("onDismiss")}
        />
    )).container;
}

beforeEach(() => {
    T.length = 0;
    h.tick = createSignal(0);
    h.entries = createSignal<unknown[]>([]);
    setEntries = h.entries[1];
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    vi.setSystemTime(T0);
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("ActivityRow, pinned", () => {
    it("lays out a running shell row, flashes on landing, ticks its elapsed time and toggles, stops and dismisses", () => {
        mount(activity({ shell: shell("npm run dev", [{ kind: "stdout", content: "ready" }]) }));
        look("running, just landed");
        advance(EXIT_FLASH_MS - 1);
        look("still landing");
        advance(1);
        look("landed");
        advance(61_000);
        look("two minutes in");
        press(".agent-activity-summary");
        press("[data-stand='IconButton:Stop']");
        setLeaving(true);
        look("leaving");
        setLeaving(false);
        setActivity(activity({ status: "done", canStop: false, endedAt: Date.now() - 1000, shell: shell("npm run dev", []) }));
        look("done");
        setActivity(activity({ status: "error", canStop: false, endedAt: Date.now(), shell: shell("npm run dev", []) }));
        look("failed");
        press("[data-stand='IconButton:Dismiss']");
        setActivity(activity({ status: "stopped", canStop: false, endedAt: Date.now(), kind: "tool" }));
        look("stopped");
        setActivity(undefined);
        look("gone");
        return snapshot("shell-row");
    });

    it("counts down a sleep to zero while it runs, and shows only the final time once it is over", () => {
        const tool = { params: { command: "sleep 30" } } as never;
        mount(activity({ kind: "tool", tool, title: "sleep 30", sleepMs: 30_000, startedAt: Date.now() }));
        look("30 s left");
        advance(20_000);
        look("10 s left");
        advance(25_000);
        look("past the deadline, clamped");
        setActivity(activity({ kind: "tool", tool, title: "sleep 30", sleepMs: 30_000, status: "done", canStop: false, startedAt: T0, endedAt: T0 + 31_000 }));
        look("over");
        return snapshot("sleep");
    });

    it("expands a shell to its full command, its log and its streaming mark, and a tool to its params, its title or no output", () => {
        mount(activity({ shell: shell("npm run dev -- --host 0.0.0.0", [{ kind: "stdout", content: "ready" }], true), title: "dev server" }), true);
        look("shell, streaming");
        setActivity(activity({ shell: shell("make", [], false), title: "build" }));
        look("shell, closed log, no chunks");
        setActivity(activity({ kind: "tool", title: "Run the tests", fullTitle: "Run the tests in full", tool: { params: { command: "npx vitest run" }, fakeChunks: [{ kind: "stdout", content: "ok" }], log: { open: true } } as never }));
        look("tool with a command and chunks");
        setActivity(activity({ kind: "tool", title: "Run the tests", fullTitle: "Run the tests in full", tool: { params: {} } as never }));
        look("tool without a command: full title, no output");
        setActivity(activity({ kind: "tool", title: "Run the tests", tool: { params: undefined, log: { open: false } } as never }));
        look("tool without params or full title: title");
        setExpanded(false);
        look("collapsed");
        return snapshot("expanded-log");
    });

    it("tails a subagent with its event count, opens its transcript only while expanded, and disposes it on collapse", () => {
        mount(activity({ kind: "subagent", title: "Scout", subagent: sub() }));
        look("no events");
        setActivity(activity({ kind: "subagent", title: "Scout", subagent: sub({ event_count: 1 }) }));
        look("one event");
        setActivity(activity({ kind: "subagent", title: "Scout", subagent: sub({ event_count: 4 }) }));
        setExpanded(true);
        look("expanded, nothing yet");
        setEntries([
            { event: { event_type: { type: "text", content: "looking" } } },
            { event: { event_type: { type: "tool_use", name: "Read" } } },
            { event: { event_type: { type: "tool_result", is_error: false, preview: "42 lines" } } },
            { event: { event_type: { type: "tool_result", is_error: true, preview: "no such file" } } },
            { event: { event_type: { type: "progress", output: "50%" } } },
            { event: { event_type: { type: "result", content: "found it" } } },
        ]);
        look("transcript");
        setExpanded(false);
        look("collapsed");
        setExpanded(true);
        setActivity(activity({ kind: "subagent", title: "Scout", subagent: sub({ dispatch_id: "d2", agent_id: "ag2", event_count: 4 }) }));
        look("another subagent while expanded");
        setActivity(undefined);
        look("gone while expanded");
        return snapshot("subagent");
    });

    it("tails a group with how many are active, and lists its members when expanded", () => {
        const members = [
            sub({ agent_id: "m1", display_name: "Alpha", status: "active", event_count: 1 }),
            sub({ agent_id: "m2", display_name: undefined, status: "completed", event_count: 3 }),
            sub({ agent_id: "m3", display_name: "Gamma", status: "abandoned", event_count: 0 }),
        ] as never[];
        mount(activity({ kind: "subagent", title: "Workflow", subagentGroup: { members } }));
        look("two of three still going");
        setExpanded(true);
        look("roster");
        setActivity(activity({ kind: "subagent", title: "Workflow", status: "done", canStop: false, subagentGroup: { members: members.slice(1) } }));
        look("none active");
        setActivity(activity({ kind: "subagent", title: "Workflow", subagentGroup: { members: [] } }));
        look("empty group");
        return snapshot("group");
    });
});
