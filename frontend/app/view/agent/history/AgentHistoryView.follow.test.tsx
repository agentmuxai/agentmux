// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The History tab follows the transcript (spec
 * SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.9): records
 * written after it loaded appear without reopening it, continue open runs
 * instead of duplicating them, are shown at most once a second, cost nothing
 * while the tab is hidden, and never leave a hole.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { Subject } from "rxjs";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentNode } from "../types";

const STREAM = "g:agent:a1:current";

/** The transcript "on disk", served by the mocked range/count reads. */
const disk = { lines: [] as string[], gen: "g1", stream: STREAM };
const reads: Array<{ offset: number; limit: number }> = [];
const counts = { n: 0 };

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        BlockfileLineCountCommand: vi.fn(async () => {
            counts.n++;
            return { count: disk.lines.length, stream: disk.stream, gen: disk.gen };
        }),
        BlockfileReadRangeCommand: vi.fn(
            async (_c: unknown, req: { offset: number; limit: number; expect_gen?: string }) => {
                reads.push({ offset: req.offset, limit: req.limit });
                if (req.expect_gen && req.expect_gen !== disk.gen) {
                    return { lines: [], stream: disk.stream, gen: disk.gen, gen_mismatch: true };
                }
                return {
                    lines: disk.lines.slice(req.offset, req.offset + req.limit),
                    total: disk.lines.length,
                    stream: disk.stream,
                    gen: disk.gen,
                };
            }
        ),
    },
}));

let subject = new Subject<unknown>();
let released = 0;
vi.mock("@/app/store/mps", () => ({
    getFileSubject: () => {
        const s = subject as Subject<unknown> & { release?: () => void };
        s.release = () => void released++;
        return s;
    },
}));

const [dormant, setDormant] = createSignal(false);
vi.mock("@/app/store/block-component-registry", () => ({ isBlockDormant: () => dormant }));
vi.mock("@/app/workspace/window-tab-visibility", () => ({ useWindowTabHidden: () => () => false }));
vi.mock("@/app/store/agent-pane-layout-store", () => ({ registerPane: () => {}, unregisterPane: () => {} }));

/** The document view, reduced to the node list it was handed. */
let shown: DocumentNode[] = [];
vi.mock("../components/AgentDocumentView", () => ({
    AgentDocumentView: (p: { documentNodes: () => DocumentNode[] }) => {
        return (
            <div>
                {(() => {
                    shown = p.documentNodes().filter((n) => n.type !== "day_divider");
                    return shown.length;
                })()}
            </div>
        );
    },
}));

import { AgentHistoryView, HISTORY_PUBLISH_MIN_INTERVAL_MS } from "./AgentHistoryView";

const ev = (event: object): string => JSON.stringify(event);
const text = (content: string) => ev({ type: "text", content });
const user = (message: string) => ev({ type: "user_message", message });

/** Write records to "disk" and emit the append event the backend would. */
function write(recs: string[], opts: { emit?: boolean } = {}): void {
    const line = disk.lines.length;
    disk.lines.push(...recs);
    if (opts.emit === false) return;
    subject.next({
        fileop: "append",
        data64: btoa(recs.map((r) => r + "\n").join("")),
        pos: [{ stream: STREAM, gen: disk.gen, line, lines: line + recs.length }],
    });
}

const flush = async (ms = 0) => {
    await vi.advanceTimersByTimeAsync(ms);
    for (let i = 0; i < 5; i++) await Promise.resolve();
};

const mount = async () => {
    render(() => (
        <AgentHistoryView blockId="hist-1" sourceBlockId="live-1" outputFormat={() => "claude-stream-json"} />
    ));
    await flush();
};

const texts = () =>
    shown.map((n) => (n.type === "markdown" ? n.content : n.type === "user_message" ? n.message : n.type));

beforeEach(() => {
    vi.useFakeTimers();
    disk.lines = [user("q1"), text("a1")];
    disk.gen = "g1";
    reads.length = 0;
    counts.n = 0;
    released = 0;
    subject = new Subject<unknown>();
    setDormant(false);
    shown = [];
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("AgentHistoryView follows the transcript", () => {
    it("shows records written after it loaded, without reopening", async () => {
        await mount();
        expect(texts()).toEqual(["q1", "a1"]);

        write([user("q2"), text("a2")]);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        expect(texts()).toEqual(["q1", "a1", "q2", "a2"]);
    });

    it("continues an open text run instead of adding a second node", async () => {
        disk.lines = [user("q1"), text("Hello")];
        await mount();
        const id = shown[1].id;

        write([text(", world")]); // text events are deltas
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        expect(shown).toHaveLength(2);
        expect(shown[1].id).toBe(id);
        expect(texts()[1]).toBe("Hello, world");
    });

    it("publishes at most once a second", async () => {
        await mount();
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS); // the load itself was a publish
        write([user("q2")]);
        await flush(0);
        expect(texts()).toEqual(["q1", "a1", "q2"]); // first change after a quiet second: at once

        write([text("a2")]);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS / 2);
        expect(texts()).toEqual(["q1", "a1", "q2"]);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS / 2);
        expect(texts()).toEqual(["q1", "a1", "q2", "a2"]);
    });

    it("fills a gap by reading it (another writer's lines)", async () => {
        await mount();
        write([user("q2")], { emit: false });
        write([text("a2")]);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        expect(texts()).toEqual(["q1", "a1", "q2", "a2"]);
    });

    it("does no work while hidden and catches up on reveal", async () => {
        await mount();
        setDormant(true);
        await flush();
        const readsBefore = reads.length;

        write([user("q2"), text("a2")]);
        write([user("q3")]);
        await flush(3 * HISTORY_PUBLISH_MIN_INTERVAL_MS);
        expect(texts()).toEqual(["q1", "a1"]);
        expect(reads.length).toBe(readsBefore);

        setDormant(false);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        expect(texts()).toEqual(["q1", "a1", "q2", "a2", "q3"]);
    });

    it("reloads the newest page when a hidden gap is too large to fill", async () => {
        await mount();
        setDormant(true);
        await flush();
        const filler = Array.from({ length: 6_000 }, (_, i) => text(`t${i}`));
        write([...filler, user("latest")]);
        setDormant(false);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);

        // A reload reads the newest page (600 lines) from the end — no
        // 6,000-line catch-up read, and no hole below the loaded range.
        const last = reads[reads.length - 1];
        expect(last.offset + last.limit).toBe(disk.lines.length);
        expect(last.limit).toBe(600);
        expect(texts().at(-1)).toBe("latest");
    });

    it("reloads instead of leaving a hole when the cursor skips a gap while shown", async () => {
        await mount();
        // Another writer appended more than the cursor will fill by reading;
        // the next event lands past it. The cursor skips — History reloads.
        write(
            Array.from({ length: 6_000 }, (_, i) => text(`t${i}`)),
            { emit: false }
        );
        write([user("latest")]);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);

        const last = reads[reads.length - 1];
        expect(last.offset + last.limit).toBe(disk.lines.length);
        expect(last.limit).toBe(600);
        expect(texts().at(-1)).toBe("latest");
    });

    it("reloads on reveal when the stream was replaced while hidden", async () => {
        await mount();
        setDormant(true);
        await flush();
        disk.lines = [user("restored")];
        disk.gen = "g2";
        subject.next({ fileop: "replace", pos: [{ stream: STREAM, gen: "g2", line: 0, lines: 1 }] });
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        expect(texts()).toEqual(["q1", "a1"]);

        setDormant(false);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        expect(texts()).toEqual(["restored"]);
    });

    it("reloads on reveal when the generation changed with no event seen", async () => {
        await mount();
        setDormant(true);
        await flush();
        write([user("q2")]); // an append noted while hidden…
        disk.lines = [user("other gen")]; // …then the stream moved on
        disk.gen = "g3";
        setDormant(false);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        expect(texts()).toEqual(["other gen"]);
    });

    it("reloads when the stream is truncated", async () => {
        await mount();
        disk.lines = [user("fresh")];
        disk.gen = "g2";
        subject.next({ fileop: "truncate" });
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        expect(texts()).toEqual(["fresh"]);
    });

    it("releases the file subject on unmount", async () => {
        await mount();
        cleanup();
        expect(released).toBe(1);
    });
});
