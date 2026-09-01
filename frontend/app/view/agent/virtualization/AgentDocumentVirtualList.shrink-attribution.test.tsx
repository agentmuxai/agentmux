// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * End-to-end wiring for per-node shrink attribution
 * (SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md step 1).
 *
 * `shrink-trace.test.ts` covers the bookkeeping in isolation. What THIS file
 * covers is the part that unit tests can't: that the streaming-buffer
 * ResizeObserver is actually attached to real rendered rows, that what it
 * records reaches `ShrinkTrace`, and that the result comes back out on the
 * `[wave-scroll-shrink]` console line. Every one of those is a wiring step
 * that can be individually correct and still not connect.
 *
 * jsdom has no layout engine and no ResizeObserver, so the fakes here mirror
 * `AgentDocumentVirtualList.resize.test.tsx`'s: `triggerResize(el)` invokes
 * whichever observer callbacks currently watch `el`, and geometry is stubbed
 * explicitly. This proves the wiring, not the browser's real resize timing.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { createAgentViewState } from "./state";
import type { DocumentNode, DocumentState } from "../types";

afterEach(() => cleanup());

type ROCallback = (entries: ResizeObserverEntry[]) => void;
let roInstances: { callback: ROCallback; targets: Set<Element> }[] = [];

class FakeResizeObserver {
    private callback: ROCallback;
    private targets = new Set<Element>();
    constructor(callback: ROCallback) {
        this.callback = callback;
        roInstances.push({ callback: this.callback, targets: this.targets });
    }
    observe(el: Element): void { this.targets.add(el); }
    unobserve(el: Element): void { this.targets.delete(el); }
    disconnect(): void { this.targets.clear(); }
}

function triggerResize(el: Element): void {
    for (const { callback, targets } of roInstances) {
        if (targets.has(el)) callback([{ target: el } as ResizeObserverEntry]);
    }
}

let rafQueue: FrameRequestCallback[] = [];
function flushRaf(): void {
    const pending = rafQueue;
    rafQueue = [];
    for (const cb of pending) cb(0);
}

interface Geometry { scrollTop: number; scrollHeight: number; clientHeight: number }

function makeScrollable(el: HTMLElement, geo: Geometry): void {
    let { scrollTop, scrollHeight, clientHeight } = geo;
    Object.defineProperty(el, "scrollTop", {
        configurable: true,
        get: () => scrollTop,
        set: (v: number) => { scrollTop = v; },
    });
    Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => scrollHeight });
    Object.defineProperty(el, "clientHeight", { configurable: true, get: () => clientHeight });
    el.scrollTo = ((opts?: ScrollToOptions | number) => {
        const requested = typeof opts === "number" ? opts : (opts?.top ?? scrollTop);
        scrollTop = Math.max(0, Math.min(requested, Math.max(0, scrollHeight - clientHeight)));
        el.dispatchEvent(new Event("scroll"));
    }) as typeof el.scrollTo;
    (el as unknown as { __setGeometry: (g: Partial<Geometry>) => void }).__setGeometry = (g) => {
        if (g.scrollHeight !== undefined) scrollHeight = g.scrollHeight;
        if (g.clientHeight !== undefined) clientHeight = g.clientHeight;
        if (g.scrollTop !== undefined) scrollTop = g.scrollTop;
    };
}

function setGeometry(el: HTMLElement, g: Partial<Geometry>): void {
    (el as unknown as { __setGeometry: (g: Partial<Geometry>) => void }).__setGeometry(g);
}

/** Force a row's measured height — `offsetHeight` is the read the sampler
 *  makes (chosen to match the pane's unzoomed `scrollHeight`; see
 *  shrink-trace.ts). jsdom reports 0 for it otherwise. */
function setRowHeight(el: Element, px: number): void {
    Object.defineProperty(el, "offsetHeight", { configurable: true, get: () => px });
}

const emptyDocumentState = (): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    expandedTools: new Set(),
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

const toolNode = (id: string): DocumentNode => ({
    type: "tool",
    id,
    tool: "Bash",
    params: { command: "echo hi" },
    status: "running",
    collapsed: false,
    summary: `Bash ${id}`,
    log: { open: true, chunks: [{ kind: "stdout", content: "out", timestamp: 1 }] },
}) as DocumentNode;

function setup(nodes: DocumentNode[]) {
    const documentAtom = createSignal<DocumentNode[]>(nodes);
    const viewState = createAgentViewState(documentAtom);
    const [docState] = createSignal(emptyDocumentState());
    const utils = render(() => (
        <AgentDocumentVirtualList
            viewState={viewState}
            documentState={docState}
            blockId="blk1234567"
            onToggleCollapse={() => {}}
            onTogglePin={() => {}}
        />
    ));
    const scrollRef = utils.container.querySelector(".agent-document") as HTMLElement;
    makeScrollable(scrollRef, { scrollTop: 0, scrollHeight: 0, clientHeight: 0 });
    const buffer = utils.container.querySelector(".agent-document-streaming-buffer") as HTMLElement;
    return { viewState, scrollRef, buffer, utils };
}

describe("AgentDocumentVirtualList — shrink attribution wiring", () => {
    let info: ReturnType<typeof vi.spyOn>;

    beforeEach(() => {
        roInstances = [];
        rafQueue = [];
        vi.stubGlobal("ResizeObserver", FakeResizeObserver);
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
            rafQueue.push(cb);
            return rafQueue.length;
        });
        vi.stubGlobal("cancelAnimationFrame", () => {});
        info = vi.spyOn(console, "info").mockImplementation(() => {});
    });

    afterEach(() => {
        vi.unstubAllGlobals();
        vi.restoreAllMocks();
    });

    /** All `[wave-scroll-shrink]` lines emitted so far, joined per call. */
    const shrinkLines = (): string[] =>
        info.mock.calls
            .filter((c) => c[0] === "[wave-scroll-shrink]")
            .map((c) => c.join(" "));

    it("names the row that shrank, with NO row-level resize notification at all", () => {
        // Regression for the P1 codex caught on PR #2887. The first version
        // fed a ResizeObserver into a time-windowed ring, but the pin path
        // that detects most shrinks runs in a `queueMicrotask`, and RO
        // callbacks are not delivered until later in the rendering steps — so
        // the primary running->terminal case logged as wholly unattributed
        // and its row shrink was left to be miscredited to a later, unrelated
        // pane delta.
        //
        // This test deliberately fires NO resize on the row itself. The only
        // thing that happens is: the DOM height changes, and a pin check runs.
        // That is the real browser ordering, and it must still attribute.
        const { scrollRef, buffer, utils } = setup([toolNode("tc-1")]);
        const row = utils.container.querySelector('[data-node-id="tc-1"]') as HTMLElement;
        expect(row).toBeTruthy();
        expect(row.dataset.nodeType).toBe("tool"); // sampler reads the kind from here

        setRowHeight(row, 300);
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 700 });
        triggerResize(buffer);
        flushRaf();

        // The tool completes: the row lays out short and the pane loses the
        // same 220px. No row resize is dispatched.
        setRowHeight(row, 80);
        setGeometry(scrollRef, { scrollHeight: 780 });
        triggerResize(buffer);
        flushRaf();

        const line = shrinkLines().pop()!;
        expect(line).toContain("delta=220px");
        expect(line).toContain("tc-1(tool) 300->80px");
        expect(line).toContain("sum=220px");
        expect(line).toContain("unattributed=0px");
    });

    it("reports an unattributed remainder when no observed row shrank", () => {
        // The pane shrank but nothing being watched did — the signal that the
        // cause is somewhere this instrumentation cannot yet see. Reporting
        // this honestly is the whole point of the remainder field.
        const { scrollRef, buffer } = setup([toolNode("tc-1")]);

        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 700 });
        triggerResize(buffer);
        flushRaf();

        setGeometry(scrollRef, { scrollHeight: 749 });
        triggerResize(buffer);
        flushRaf();

        const line = shrinkLines().pop()!;
        expect(line).toContain("delta=251px");
        expect(line).toContain("attributed: none");
        expect(line).toContain("unattributed=251px");
    });

    it("sums several rows into one pane delta", () => {
        // The case a bare net delta cannot decompose, and the reason the
        // 08-22 findings' ~251-252px lead had no matching height constant.
        const { scrollRef, buffer, utils } = setup([toolNode("tc-1"), toolNode("tc-2")]);
        const r1 = utils.container.querySelector('[data-node-id="tc-1"]') as HTMLElement;
        const r2 = utils.container.querySelector('[data-node-id="tc-2"]') as HTMLElement;

        setRowHeight(r1, 200);
        setRowHeight(r2, 100);
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 700 });
        triggerResize(buffer);
        flushRaf();

        setRowHeight(r1, 60); // -140
        setRowHeight(r2, 12); // -88
        setGeometry(scrollRef, { scrollHeight: 749 }); // pane lost 251
        triggerResize(buffer);
        flushRaf();

        const line = shrinkLines().pop()!;
        expect(line).toContain("tc-1(tool) 200->60px");
        expect(line).toContain("tc-2(tool) 100->12px");
        expect(line).toContain("sum=228px");
        expect(line).toContain("unattributed=23px"); // 251 seen, 228 explained
    });

    it("logs nothing when the pane did not shrink", () => {
        const { scrollRef, buffer } = setup([toolNode("tc-1")]);
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 700 });
        triggerResize(buffer);
        flushRaf();
        setGeometry(scrollRef, { scrollHeight: 1400 }); // grew
        triggerResize(buffer);
        flushRaf();
        expect(shrinkLines()).toEqual([]);
    });
});
