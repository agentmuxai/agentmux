// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActivityDock, pinned as a whole ahead of splitting it into a state object and view parts: each scenario mounts the
 * real dock with stand-ins for the adapters that feed it (the shells, the subagents, the tool calls, the registry
 * rows and the stream outcomes), the clocks, the row, the overflow button, the stop RPC and the naming request, and
 * logs the dock as an indented tree and, in order, every hook it builds and every call its rows make. The logs are
 * snapshots recorded on the unsplit component; the split must leave them byte for byte. (Nothing tested this dock
 * before: this pins its ordering, its cap, its retention and its callbacks.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { calls as T } from "./agent-bottom-panels.behavior.stub";

const h = vi.hoisted(() => ({ tick: null as any, nonce: null as any, shells: null as any, subs: null as any, registry: null as any, stop: null as any }));
const log = (line: string): void => {
    T.push(line);
};

vi.mock("@/app/hook/useTick", () => ({ useTick: (ms: number) => (log(`useTick ${ms}`), h.tick[0]) }));
vi.mock("../activity/promotion-clock", () => ({ createPromotionClock: () => (log("createPromotionClock"), h.nonce[0]) }));
vi.mock("../../../agent-model/task-outcomes", () => ({
    taskOutcomesFor: (blockId: string) => (log(`taskOutcomesFor ${blockId}`), () => ({})),
    applyStreamOutcomes: (acts: unknown[]) => (log(`applyStreamOutcomes ${acts.length}`), acts),
}));
vi.mock("../activity/background-adapter", () => ({
    applyRegistryOutcomes: (acts: unknown[], tasks: unknown[]) => (log(`applyRegistryOutcomes ${acts.length} ${tasks.length}`), acts),
    backgroundTaskActivities: (tasks: unknown[], known: Set<string>) => {
        log(`backgroundTaskActivities ${tasks.length} known=[${[...known].join(",")}]`);
        return (tasks as { id: string }[]).filter((t) => !known.has(t.id)).map((t) => ({ ...t, kind: "shell", canStop: false }));
    },
}));
vi.mock("../activity/shell-adapter", () => ({ shellActivities: () => (log("shellActivities"), h.shells[0]()) }));
vi.mock("../activity/subagent-adapter", () => ({ subagentActivities: (_: unknown, blockId: string) => (log(`subagentActivities ${blockId}`), h.subs[0]()) }));
vi.mock("../activity/subagent-source", () => ({ allSubagentsAtom: () => [] }));
vi.mock("../../../agent-model/tool-adapter", () => ({ toolActivities: () => (log("toolActivities"), []) }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ShellStopCommand: (_c: unknown, d: { shell_id: string }) => (log(`ShellStopCommand ${d.shell_id}`), h.stop(d.shell_id)) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/swarm/subagent-naming", () => ({ requestSubagentName: (id: string) => log(`requestSubagentName ${id}`) }));
vi.mock("./ActivityRow", () => ({
    ActivityRow: (p: { activity: () => any; expanded: () => boolean; leaving: () => boolean; onToggle: () => void; onStop: () => void; onDismiss: () => void }) => (
        <div data-stand="ActivityRow" data-id={p.activity()?.id} data-status={p.activity()?.status} data-expanded={String(p.expanded())} data-leaving={String(p.leaving())}>
            <span data-cb="onToggle" onClick={() => (log(`ActivityRow ${p.activity()?.id} onToggle`), p.onToggle())}>
                toggle
            </span>
            <span data-cb="onStop" onClick={() => (log(`ActivityRow ${p.activity()?.id} onStop`), p.onStop())}>
                stop
            </span>
            <span data-cb="onDismiss" onClick={() => (log(`ActivityRow ${p.activity()?.id} onDismiss`), p.onDismiss())}>
                dismiss
            </span>
        </div>
    ),
}));
vi.mock("@/app/element/ui/Button", () => ({
    Button: (p: { variant: string; density: string; class: string; onClick: () => void; children?: JSX.Element }) => (
        <span data-stand="Button" data-variant={p.variant} data-density={p.density} data-class={p.class} data-cb="onClick" onClick={() => (log("Button onClick"), p.onClick())}>
            {p.children}
        </span>
    ),
}));

import { ActivityDock } from "./ActivityDock";
import type { PinnedActivity } from "../../../agent-model/activity-types";

const T0 = 1_800_000_000_000;
const SNAP = (name: string) => `./__snapshots__/ActivityDock.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type)$/;

/** The dock as an indented tree: tags, classes, the attributes that carry state, and text. */
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
let setRegistry: (v: unknown[]) => void;
let setShells: (v: PinnedActivity[]) => void;
let setSubs: (v: PinnedActivity[]) => void;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const advance = (ms: number) => {
    vi.advanceTimersByTime(ms);
    h.tick[1]((n: number) => n + 1);
};
const press = (id: string | null, cb: string) => {
    const sel = id ? `[data-stand="ActivityRow"][data-id="${id}"] [data-cb="${cb}"]` : `[data-stand="Button"]`;
    fireEvent.click(host.querySelector(sel)!);
};

let n = 0;
const act = (id: string, over: Partial<PinnedActivity> = {}): PinnedActivity => ({
    id,
    kind: "shell",
    title: id,
    status: "running",
    startedAt: T0 - 60_000 + n++ * 1000,
    canStop: true,
    ...over,
});
const named = (name: string | undefined, agent_id: string) => ({ display_name: name, agent_id }) as never;

function mount(shells: PinnedActivity[] = [], subs: PinnedActivity[] = []) {
    h.shells = createSignal(shells);
    h.subs = createSignal(subs);
    h.registry = createSignal<unknown[]>([]);
    setShells = h.shells[1];
    setSubs = h.subs[1];
    setRegistry = h.registry[1];
    host = render(() => <ActivityDock documentNodes={() => []} blockId="blk-1" backgroundTasksAtom={() => h.registry[0]() as never} />).container;
}

beforeEach(() => {
    T.length = 0;
    n = 0;
    h.tick = createSignal(0);
    h.nonce = createSignal(0);
    h.stop = () => Promise.resolve();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    vi.setSystemTime(T0);
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("ActivityDock, pinned", () => {
    it("builds its memos in order, shows nothing while there are no activities, and orders running first, then error, stopped, done, newest first", () => {
        mount();
        look("no activities");
        setShells([
            act("done-1", { status: "done", canStop: false, endedAt: T0 }),
            act("run-old"),
            act("err-1", { status: "error", canStop: false, endedAt: T0 }),
            act("stopped-1", { status: "stopped", canStop: false, endedAt: T0 }),
        ]);
        setSubs([act("run-new", { kind: "subagent" })]);
        look("mixed");
        return snapshot("order");
    });

    it("keeps three rows inline, summarises the rest by kind, opens them on the button, and keeps an expanded row past the cap", () => {
        mount(
            [act("s1"), act("s2"), act("s3"), act("s4"), act("c1", { kind: "cron" })],
            [act("u1", { kind: "subagent" }), act("t1", { kind: "tool" })],
        );
        look("three inline, four more");
        press(null, "onClick");
        look("overflow open");
        press("s2", "onToggle");
        press(null, "onClick");
        look("closed, the expanded row stays");
        press("s2", "onToggle");
        look("collapsed again");
        setShells([act("s1"), act("s2")]);
        setSubs([]);
        look("back under the cap");
        return snapshot("overflow");
    });

    it("keeps a finished row for its retention window, flags it as leaving, removes it after the flash and never drops a running one", () => {
        mount([act("run"), act("done", { status: "done", canStop: false, endedAt: Date.now() }), act("err", { status: "error", canStop: false, endedAt: Date.now() })]);
        look("just finished");
        advance(7_900);
        look("done is nearly out");
        advance(200);
        look("done is leaving");
        advance(300);
        look("done is gone");
        advance(7_000);
        look("error is nearly out");
        advance(1_000);
        look("everything finished is gone");
        setShells([act("run"), act("done2", { status: "done", canStop: false, endedAt: Date.now() })]);
        look("a new finish");
        advance(60_000);
        look("a minute on");
        return snapshot("retention");
    });

    it("dismisses a row for good, stops only shells and crons and ignores a stop that fails", () => {
        h.stop = (id: string) => (id === "bad" ? Promise.reject(new Error("gone")) : Promise.resolve());
        mount([act("sh"), act("bad"), act("cr", { kind: "cron" }), act("er", { status: "error", canStop: false, endedAt: Date.now() })], [act("sa", { kind: "subagent" })]);
        press(null, "onClick");
        press("sh", "onStop");
        press("bad", "onStop");
        press("cr", "onStop");
        press("sa", "onStop");
        press("er", "onDismiss");
        look("after the stops and the dismissal");
        setShells([act("sh"), act("bad"), act("cr"), act("er", { status: "error", canStop: false, endedAt: Date.now() })]);
        look("the dismissed row does not come back");
        return snapshot("actions");
    });

    it("asks for a subagent's name on its first expand only, and for every unnamed member of a group", () => {
        mount(
            [],
            [
                act("unnamed", { kind: "subagent", subagent: named(undefined, "ag-unnamed") }),
                act("named", { kind: "subagent", subagent: named("Scout", "ag-named") }),
                act("group", { kind: "subagent", subagentGroup: { members: [named("Alpha", "m1"), named(undefined, "m2"), named(undefined, "m3")] } }),
                act("plain", { kind: "tool" }),
            ],
        );
        press(null, "onClick");
        press("unnamed", "onToggle");
        look("unnamed expanded");
        press("unnamed", "onToggle");
        look("unnamed collapsed: no request");
        press("unnamed", "onToggle");
        press("named", "onToggle");
        press("group", "onToggle");
        press("plain", "onToggle");
        look("all expanded");
        return snapshot("naming");
    });

    it("fills in registry rows the transcript did not produce, and drops the ones it did", () => {
        mount([act("known")]);
        setRegistry([{ id: "known", title: "known", status: "running", startedAt: T0 }, { id: "survivor", title: "survivor", status: "running", startedAt: T0 - 5000 }]);
        look("registry rows");
        setRegistry([]);
        look("registry empty");
        return snapshot("registry");
    });
});
