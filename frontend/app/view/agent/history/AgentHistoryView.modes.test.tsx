// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Agent History reader's modes (SPEC_WARDEN_HISTORY_SECTION_2026_10_18.md
 * phase 2): an archived conversation is read through `archive_zone` and isn't
 * followed; a jump shows the conversation up to a line; Show newer pages
 * forward; Back to latest follows the current conversation again.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { Subject } from "rxjs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentNode } from "../types";
import type { HistoryNavigatorProps } from "./HistoryNavigator";

const STREAM = "g:agent:a1:current";
const ARCHIVE = "agent:a1:archive:1000";
const ev = (event: object): string => JSON.stringify(event);
const text = (content: string) => ev({ type: "text", content });

const current = Array.from({ length: 1500 }, (_, i) => text(`current ${i}`));
const archived = [text("archived 0"), text("archived 1")];
const reads: Array<{ offset: number; limit: number; archive_zone?: string }> = [];

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        BlockfileLineCountCommand: vi.fn(async (_c: unknown, req: { archive_zone?: string }) => ({
            count: (req.archive_zone ? archived : current).length,
            stream: STREAM,
            gen: "g1",
        })),
        BlockfileReadRangeCommand: vi.fn(async (_c: unknown, req: { offset: number; limit: number; archive_zone?: string }) => {
            reads.push({ offset: req.offset, limit: req.limit, archive_zone: req.archive_zone });
            const lines = req.archive_zone ? archived : current;
            return { lines: lines.slice(req.offset, req.offset + req.limit), total: lines.length, stream: STREAM, gen: "g1" };
        }),
    },
}));

const subject = new Subject<unknown>();
vi.mock("@/app/store/mps", () => ({
    getFileSubject: () => Object.assign(subject, { release: () => {} }),
}));
vi.mock("@/app/store/block-component-registry", () => ({ isBlockDormant: () => () => false }));
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
vi.mock("./HistoryNavigator", () => ({
    HistoryNavigator: (p: HistoryNavigatorProps) => {
        nav = p;
        return null;
    },
}));

import { AgentHistoryView } from "./AgentHistoryView";

/** What is shown, as one string: consecutive text records merge into one node. */
const shownText = (): string => shown.map((n) => (n as { content?: string }).content ?? "").join("");
const settle = async () => {
    await vi.advanceTimersByTimeAsync(2000);
    for (let i = 0; i < 10; i++) await Promise.resolve();
};

describe("AgentHistoryView modes", () => {
    beforeEach(async () => {
        vi.useFakeTimers();
        reads.length = 0;
        nav = null;
        render(() => <AgentHistoryView blockId="hist-1" sourceBlockId="live-1" agentId="a1" outputFormat={() => "claude-stream-json"} />);
        await settle();
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("opens live, on the newest page", () => {
        expect(nav!.mode()).toBe("live");
        expect(shownText().startsWith("current 900")).toBe(true);
        expect(shownText().endsWith("current 1499")).toBe(true);
    });

    it("an archive is read through archive_zone, and appends to the current conversation stay out of it", async () => {
        nav!.onOpenArchive(ARCHIVE);
        await settle();
        expect(nav!.mode()).toBe("archive");
        expect(reads.at(-1)?.archive_zone).toBe(ARCHIVE);
        expect(shownText()).toBe("archived 0archived 1");
        subject.next({ fileop: "append", data64: btoa(text("live append") + "\n"), pos: [{ stream: STREAM, gen: "g1", line: 1500, lines: 1501 }] });
        await settle();
        expect(shownText()).toBe("archived 0archived 1");
    });

    it("a jump to before the first line shows nothing, and says so", async () => {
        nav!.onJump(0);
        await settle();
        expect(shownText()).toBe("");
        expect(screen.getByText("nothing before this point")).toBeTruthy();
    });

    it("a jump shows the conversation up to the line, Show newer pages forward, Back to latest follows again", async () => {
        nav!.onJump(700);
        await settle();
        expect(nav!.mode()).toBe("jumped");
        expect(shownText().startsWith("current 100")).toBe(true);
        expect(shownText().endsWith("current 699")).toBe(true);
        expect(nav!.hasNewer()).toBe(true);

        nav!.onShowNewer();
        await settle();
        expect(shownText().endsWith("current 1299")).toBe(true);

        nav!.onLatest();
        await settle();
        expect(nav!.mode()).toBe("live");
        expect(shownText().endsWith("current 1499")).toBe(true);
    });
});
