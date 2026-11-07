// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPanePerfSection (the diag panel's agent-pane perf block), pinned as a whole ahead of splitting it into a
 * snapshot-polling hook and one small view per table or list: each scenario mounts the real section with a stand-in
 * for the perf store (its snapshot is whatever the scenario sets, and every read is logged) and a fixed clock, lets
 * the one-second poll run, and logs the section as an indented tree (inline styles included) and, in order, every
 * call it makes. The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({ snap: null as any }));

vi.mock("@/app/agent-model/perf-probe", () => ({
    ESTIMATOR_MISS_THRESHOLD: 0.3,
    agentPerfStore: {
        snapshot: () => (log("agentPerfStore.snapshot"), h.snap),
    },
}));

import { AgentPanePerfSection } from "./agent-pane-perf-section";

const SNAP = (name: string) => `./__snapshots__/agent-pane-perf-section.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|value|placeholder)$/;

/** The section as an indented tree: tags, classes, the attributes that carry state (styles included), and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

const NOW = 100_000;
let host: HTMLElement;
let unmount: () => void;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const poll = (times = 1) => {
    log(`poll x${times}`);
    vi.advanceTimersByTime(1000 * times);
};

const empty = () => ({
    rowMountByKind: new Map(),
    estimatorMissRateByKind: new Map(),
    recentEstimatorMisses: [] as any[],
    recentLayoutShifts: [] as any[],
    dispatchByKind: new Map(),
    transcriptCursors: [] as any[],
});
const stats = (over: Record<string, number> = {}) => ({
    delivered: 10,
    duplicates: 1,
    gapsFilled: 2,
    gapLinesFilled: 5,
    linesSkipped: 0,
    unpositioned: 0,
    genChanges: 0,
    echoes: 3,
    ownEchoesDropped: 4,
    ...over,
});

function mount() {
    const view = render(() => <AgentPanePerfSection />);
    host = view.container;
    unmount = view.unmount;
}

beforeEach(() => {
    T.length = 0;
    h.snap = empty();
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    vi.spyOn(performance, "now").mockReturnValue(NOW);
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe("AgentPanePerfSection, pinned", () => {
    it("renders nothing while the store has no data, shows the block once a poll finds some, hides it again when it goes, and stops polling on unmount", () => {
        mount();
        look("no data at mount");
        h.snap = { ...empty(), recentLayoutShifts: [{ value: 0.01234, timestamp: NOW - 500 }] };
        look("data set, no poll yet");
        poll();
        look("a poll later: the block appears");
        h.snap = empty();
        poll();
        look("data gone, next poll: hidden again");
        unmount();
        log("unmounted");
        poll(3);
        return snapshot("visibility");
    });

    it("lists row mount times by kind, slowest p95 first, with ms formatting, the estimator-miss rate and its flag", () => {
        h.snap = {
            ...empty(),
            rowMountByKind: new Map<string, any>([
                ["text", { p50: 1.23, p95: 4.56, max: 9.99, count: 64 }],
                ["tool-call", { p50: 12.4, p95: 88.5, max: 120.2, count: 30 }],
                ["thinking", { p50: undefined, p95: undefined, max: undefined, count: 2 }],
                ["diff", { p50: 9.95, p95: 10, max: 10.4, count: 7 }],
                ["image", { p50: 0, p95: 4.56, max: 5, count: 1 }],
            ]),
            estimatorMissRateByKind: new Map<string, number>([
                ["text", 0.04],
                ["tool-call", 0.2],
                ["diff", 0.2001],
                ["thinking", 0.55],
            ]),
        };
        mount();
        look("row mount table");
        return snapshot("row-mounts");
    });

    it("lists store dispatch times by kind, slowest max first", () => {
        h.snap = {
            ...empty(),
            dispatchByKind: new Map<string, any>([
                ["layout", { p50: 0.4, p95: 1, max: 2.5, count: 64 }],
                ["document", { p50: 3.21, p95: 9, max: 41.7, count: 12 }],
                ["other", { p50: undefined, p95: undefined, max: undefined, count: 0 }],
            ]),
        };
        mount();
        look("dispatch table");
        return snapshot("dispatches");
    });

    it("lists transcript cursors per pane with their counters, flagging the ones that skipped, left unpositioned or changed generation", () => {
        h.snap = {
            ...empty(),
            transcriptCursors: [
                { blockId: "0123456789abcdef", stats: stats() },
                { blockId: "short", stats: stats({ linesSkipped: 2 }) },
                { blockId: "ffffffff-1111", stats: stats({ unpositioned: 1, genChanges: 3 }) },
            ],
        };
        mount();
        look("cursors");
        return snapshot("cursors");
    });

    it("lists the newest five layout shifts and the newest five estimator misses with their ages in ms, seconds and minutes", () => {
        h.snap = {
            ...empty(),
            recentLayoutShifts: [
                { value: 0.1, timestamp: NOW - 999 },
                { value: 0.02, timestamp: NOW - 1000 },
                { value: 0.003, timestamp: NOW - 59_999 },
                { value: 0.0004, timestamp: NOW - 60_000 },
                { value: 1, timestamp: NOW - 150_000 },
                { value: 5, timestamp: NOW - 200_000 },
                { value: 6, timestamp: NOW - 300_000 },
            ],
            recentEstimatorMisses: [
                { kind: "text", estimated: 100, actual: 150, errorPct: 0.5, timestamp: NOW - 10 },
                { kind: "tool-call", estimated: 80.4, actual: 33.6, errorPct: 0.582, timestamp: NOW - 2500 },
                { kind: "diff", estimated: 10, actual: 30, errorPct: 2, timestamp: NOW - 90_000 },
                { kind: "image", estimated: 1, actual: 2, errorPct: 1, timestamp: NOW - 91_000 },
                { kind: "text", estimated: 2, actual: 3, errorPct: 0.5, timestamp: NOW - 92_000 },
                { kind: "thinking", estimated: 3, actual: 4, errorPct: 0.33, timestamp: NOW - 93_000 },
            ],
        };
        mount();
        look("shifts and misses");
        return snapshot("recent");
    });

    it("shows every block together, in order, and follows the store on each poll", () => {
        h.snap = {
            rowMountByKind: new Map<string, any>([["text", { p50: 2, p95: 3, max: 4, count: 5 }]]),
            estimatorMissRateByKind: new Map<string, number>([["text", 0.5]]),
            recentEstimatorMisses: [{ kind: "text", estimated: 10, actual: 20, errorPct: 1, timestamp: NOW - 1 }],
            recentLayoutShifts: [{ value: 0.5, timestamp: NOW - 2 }],
            dispatchByKind: new Map<string, any>([["layout", { p50: 1, p95: 2, max: 3, count: 4 }]]),
            transcriptCursors: [{ blockId: "abcdefghijkl", stats: stats() }],
        };
        mount();
        look("everything");
        h.snap = {
            ...h.snap,
            rowMountByKind: new Map<string, any>([["diff", { p50: 20, p95: 30, max: 40, count: 50 }]]),
            transcriptCursors: [],
            recentLayoutShifts: [],
        };
        poll();
        look("after a poll: new row kind, no cursors, no shifts");
        return snapshot("everything");
    });
});
