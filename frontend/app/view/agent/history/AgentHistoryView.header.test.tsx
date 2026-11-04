// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the History reader's header says in each state, and the poll that catches lines no event
 * brings (another writer to the agent's shared zone). The follow and modes tests cover the reader's
 * behaviour; this pins its words and its poll, so splitting the view can't change either.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { Emitter } from "@/util/emitter";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentNode } from "../../../agent-model/types";
import type { HistoryNavigatorProps } from "./HistoryNavigator";

const STREAM = "g:agent:a1:current";
const ARCHIVE = "agent:a1:archive:1000";
const ev = (event: object): string => JSON.stringify(event);
const text = (content: string) => ev({ type: "text", content });
const user = (message: string) => ev({ type: "user_message", message });

const disk = { lines: [] as string[], archived: [] as string[], gen: "g1", stream: STREAM, failCount: false };
const counts = { n: 0 };

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        BlockfileLineCountCommand: vi.fn(async (_c: unknown, req: { archive_zone?: string }) => {
            counts.n++;
            if (disk.failCount) throw new Error("srv is down");
            return { count: (req.archive_zone ? disk.archived : disk.lines).length, stream: disk.stream, gen: disk.gen };
        }),
        BlockfileReadRangeCommand: vi.fn(async (_c: unknown, req: { offset: number; limit: number; archive_zone?: string }) => {
            const lines = req.archive_zone ? disk.archived : disk.lines;
            return { lines: lines.slice(req.offset, req.offset + req.limit), total: lines.length, stream: disk.stream, gen: disk.gen };
        }),
    },
}));

let subject = new Emitter<unknown>();
vi.mock("@/app/store/mps", () => ({ getFileSubject: () => Object.assign(subject, { release: () => {} }) }));
const [dormant, setDormant] = createSignal(false);
vi.mock("@/app/store/block-component-registry", () => ({ isBlockDormant: () => dormant }));
vi.mock("@/app/workspace/window-tab-visibility", () => ({ useWindowTabDisplayed: () => () => true }));
vi.mock("@/app/store/agent-pane-layout-store", () => ({ registerPane: () => {}, unregisterPane: () => {} }));

let shown: DocumentNode[] = [];
vi.mock("../components/AgentDocumentView", () => ({
    AgentDocumentView: (p: { documentNodes: () => DocumentNode[] }) => (
        <div>
            {(() => {
                shown = p.documentNodes().filter((n) => n.type !== "day_divider");
                return shown.length;
            })()}
        </div>
    ),
}));
let nav: HistoryNavigatorProps | null = null;
vi.mock("./HistoryNavigator", async (importOriginal) => ({
    ...(await importOriginal<typeof import("./HistoryNavigator")>()),
    HistoryNavigator: (p: HistoryNavigatorProps) => {
        nav = p;
        return null;
    },
}));

import { AgentHistoryView, HISTORY_PAGE } from "./AgentHistoryView";

const flush = async (ms = 0) => {
    await vi.advanceTimersByTimeAsync(ms);
    for (let i = 0; i < 10; i++) await Promise.resolve();
};
const mount = () => render(() => <AgentHistoryView blockId="hist-1" sourceBlockId="live-1" agentId="a1" outputFormat={() => "claude-stream-json"} />);
const meta = (): string => document.querySelector(".agent-history-meta")?.textContent ?? "";
const texts = () => shown.map((n) => (n.type === "markdown" ? n.content : n.type === "user_message" ? n.message : n.type));

beforeEach(() => {
    vi.useFakeTimers();
    disk.lines = [user("q1"), text("a1")];
    disk.archived = [text("old 0"), text("old 1"), text("old 2")];
    disk.gen = "g1";
    disk.failCount = false;
    counts.n = 0;
    subject = new Emitter<unknown>();
    setDormant(false);
    shown = [];
    nav = null;
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("the header's meta line", () => {
    it("says it is loading until the first page arrives", async () => {
        mount();
        expect(meta()).toBe("loading history…");
        await flush();
        expect(meta()).not.toBe("loading history…");
    });

    it("says why a page could not be read", async () => {
        disk.failCount = true;
        mount();
        await flush();
        expect(meta()).toBe("couldn't load history: srv is down");
    });

    it("says there is no history for an empty transcript", async () => {
        disk.lines = [];
        mount();
        await flush();
        expect(meta()).toBe("no recorded history");
    });

    it("says the whole transcript is loaded when it fits one page", async () => {
        mount();
        await flush();
        expect(meta()).toBe("full history loaded");
    });

    it("counts the lines and points up when older ones remain", async () => {
        disk.lines = Array.from({ length: HISTORY_PAGE + 40 }, (_, i) => text(`t${i}`));
        mount();
        await flush();
        expect(meta()).toBe(`${HISTORY_PAGE + 40} lines · scroll up for earlier`);
    });

    it("names the lines a jump shows, and an archive says so", async () => {
        mount();
        await flush();
        nav!.onJump(1);
        await flush();
        expect(meta()).toBe("lines 1–1 of 2");
        nav!.onOpenArchive(ARCHIVE);
        await flush();
        expect(meta()).toBe("archived conversation · lines 1–3 of 3");
        nav!.onJump(0);
        await flush();
        expect(meta()).toBe("nothing before this point");
    });
});

describe("the poll for lines no event brings", () => {
    it("asks for the line count every five seconds while shown, and shows lines it finds on the second look", async () => {
        mount();
        await flush();
        const before = counts.n;
        // Another writer appended: no event, only the count moves.
        disk.lines.push(user("q2"));
        await flush(5_000);
        expect(counts.n).toBe(before + 1);
        expect(texts()).toEqual(["q1", "a1"]);
        await flush(5_000);
        expect(counts.n).toBe(before + 2);
        await flush(1_000);
        expect(texts()).toEqual(["q1", "a1", "q2"]);
    });

    it("does not ask while the tab is hidden", async () => {
        mount();
        await flush();
        setDormant(true);
        await flush();
        const before = counts.n;
        await flush(20_000);
        expect(counts.n).toBe(before);
    });

    it("starts over when the stream was recreated under a new generation", async () => {
        mount();
        await flush();
        disk.lines = [user("fresh")];
        disk.gen = "g7";
        await flush(5_000);
        await flush(1_500);
        expect(texts()).toEqual(["fresh"]);
    });

    it("stops asking once the view is gone", async () => {
        const r = mount();
        await flush();
        r.unmount();
        const before = counts.n;
        await flush(20_000);
        expect(counts.n).toBe(before);
    });
});
