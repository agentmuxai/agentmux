// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What AgentHistoryView does, pinned as ordered logs: the order it sets itself up in, the reads it
 * makes and what it shows after each, a hidden tab and its catch-up, the ways it recovers from a hole,
 * jumps and archives, a requested position, paging and the wheel. The other test files in this folder
 * hold the follow, header and mode behaviour; this keeps the rest from changing in a refactor of the
 * view. Regenerate the committed snapshots with `vitest -u` only for a change meant to differ.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { Emitter } from "@/util/emitter";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentNode } from "../../../agent-model/types";

const STREAM = "g:agent:a1:current";
const T = vi.hoisted(() => ({ trace: [] as string[] }));
const log = (line: string) => T.trace.push(line);
const out = () => T.trace.join("\n") + "\n";
const snap = (name: string) => `./__snapshots__/AgentHistoryView.behavior.${name}.txt`;

/** The transcript "on disk" and what the reads fail with. */
const disk = { lines: [] as string[], gen: "g1", stream: STREAM, zones: {} as Record<string, string[]> };
const fail = { count: false, range: false, locate: false, feed: false };
/** A read held open until the test lets it finish. */
const gate = { hold: { count: false, range: false }, waiting: { count: [] as (() => void)[], range: [] as (() => void)[] } };
const held = (kind: "count" | "range") => (gate.hold[kind] ? new Promise<void>((r) => gate.waiting[kind].push(r)) : undefined);
const release = async (kind: "count" | "range") => {
    gate.waiting[kind].splice(0).forEach((r) => r());
    for (let i = 0; i < 8; i++) await Promise.resolve();
};
const ev = (event: object): string => JSON.stringify(event);
const text = (content: string) => ev({ type: "text", content });
const user = (message: string) => ev({ type: "user_message", message });
const fmt = (v: unknown): string => JSON.stringify(v, (_k, x) => (typeof x === "function" ? "fn" : x)) ?? "undefined";

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        BlockfileLineCountCommand: vi.fn(async (_c: unknown, req: { archive_zone?: string }, opts: { timeout: number }) => {
            log(`count(${fmt({ ...req, block_id: undefined })}, timeout ${opts.timeout})`);
            await held("count");
            if (fail.count) throw new Error("count failed");
            const lines = req.archive_zone ? (disk.zones[req.archive_zone] ?? []) : disk.lines;
            return { count: lines.length, stream: disk.stream, gen: disk.gen };
        }),
        BlockfileReadRangeCommand: vi.fn(async (_c: unknown, req: { offset: number; limit: number; archive_zone?: string }, opts: { timeout: number }) => {
            log(`range(${fmt({ offset: req.offset, limit: req.limit, archive_zone: req.archive_zone })}, timeout ${opts.timeout})`);
            await held("range");
            if (fail.range) throw new Error("range failed");
            const lines = req.archive_zone ? (disk.zones[req.archive_zone] ?? []) : disk.lines;
            return { lines: lines.slice(req.offset, req.offset + req.limit), total: lines.length, stream: disk.stream, gen: disk.gen };
        }),
        HistoryLocateCommand: vi.fn(async (_c: unknown, req: object) => {
            log(`locate(${fmt({ ...req, block_id: undefined })})`);
            if (fail.locate) throw new Error("locate failed");
            return { line: 40 };
        }),
    },
}));

let subject = new Emitter<unknown>();
vi.mock("@/app/store/mps", () => ({
    getFileSubject: (id: string, name: string) => {
        log(`getFileSubject(${id}, ${name})`);
        const real = subject;
        return {
            subscribe: (fn: (e: unknown) => void) => {
                log("subscribe");
                const s = real.subscribe(fn);
                return { unsubscribe: () => (log("unsubscribe"), s.unsubscribe()) };
            },
            release: () => log("release"),
        };
    },
}));

const [vis, setVis] = createSignal<"active" | "dormant">("active");
vi.mock("@/app/block/pane-tab-visibility", () => ({ usePaneTabVisibility: (id: string) => (log(`usePaneTabVisibility(${id})`), vis) }));
vi.mock("@/app/store/agent-pane-layout-store", () => ({
    registerPane: (key: string) => log(`registerPane(${key})`),
    unregisterPane: (key: string) => log(`unregisterPane(${key})`),
}));
vi.mock("./history-poll", () => ({
    startHistoryPoll: (p: Record<string, unknown>) => {
        log(`startHistoryPoll(${Object.keys(p).sort().join(",")})`);
        h.poll = p;
        return () => log("poll stopped");
    },
}));
vi.mock("../parseHistoryLines", async (orig) => {
    const real = await orig<typeof import("../parseHistoryLines")>();
    return {
        ...real,
        HistoryParser: class extends real.HistoryParser {
            feed(lines: string[], stamps?: (number | undefined)[]) {
                if (fail.feed) throw new Error("parse failed");
                return super.feed(lines, stamps as number[]);
            }
        },
    };
});

const h: { poll: any; doc: any; nav: any; scroller: HTMLElement | null; scrolls: string[] } = { poll: undefined, doc: undefined, nav: undefined, scroller: null, scrolls: [] };
let shown: DocumentNode[] = [];
vi.mock("../components/AgentDocumentView", () => ({
    AgentDocumentView: (p: any) => {
        h.doc = p;
        p.followingRef?.(following);
        p.scrollToBottomRef?.((reason?: string) => log(`scrollToBottom(${reason})`));
        return (
            <div class="agent-document" ref={(el) => (h.scroller = el)}>
                {(() => {
                    shown = p.documentNodes().filter((n: DocumentNode) => n.type !== "day_divider");
                    return shown.length;
                })()}
            </div>
        );
    },
}));
vi.mock("./HistoryNavigator", async (orig) => ({
    ...(await orig<typeof import("./HistoryNavigator")>()),
    HistoryNavigator: (p: any) => {
        h.nav = p;
        return <div class="stub-navigator" />;
    },
}));
const [following, setFollowing] = createSignal(true);

import { AgentHistoryView, HISTORY_PAGE, HISTORY_PUBLISH_MIN_INTERVAL_MS } from "./AgentHistoryView";

const flush = async (ms = 0) => {
    await vi.advanceTimersByTimeAsync(ms);
    for (let i = 0; i < 8; i++) await Promise.resolve();
};
const texts = () =>
    shown.map((n) => {
        const t = n.type === "markdown" ? n.content : n.type === "user_message" ? n.message : n.type;
        return t.length > 24 ? `${t.slice(0, 24)}…(${t.length})` : t;
    });
const meta = (c: HTMLElement) => c.querySelector(".agent-history-meta")?.textContent;
const state = (c: HTMLElement, label: string) => log(`${label}: shown=${fmt(texts())} meta=${JSON.stringify(meta(c))} jumpButton=${!!c.querySelector(".agent-history-jump-latest")}`);

function mount(over: Record<string, unknown> = {}) {
    const r = render(() => <AgentHistoryView blockId="hist-1" sourceBlockId="live-1" agentId="a1" outputFormat={() => "claude-stream-json"} agentName={() => "korp"} {...(over as object)} />);
    return r;
}
function write(recs: string[], emit = true): void {
    const line = disk.lines.length;
    disk.lines.push(...recs);
    if (!emit) return;
    subject.next({ fileop: "append", data64: btoa(recs.map((r) => r + "\n").join("")), pos: [{ stream: STREAM, gen: disk.gen, line, lines: line + recs.length }] });
}

beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => setTimeout(() => cb(0), 16) as unknown as number);
    T.trace.length = 0;
    vi.spyOn(console, "warn").mockImplementation((...a: unknown[]) => void log(`warn: ${String(a[0])}`));
    disk.lines = [user("q1"), text("a1")];
    disk.gen = "g1";
    disk.zones = { "agent:a1:archive:1": [text("old 0"), user("old 1")] };
    Object.assign(fail, { count: false, range: false, locate: false, feed: false });
    gate.hold = { count: false, range: false };
    gate.waiting = { count: [], range: [] };
    subject = new Emitter<unknown>();
    setVis("active");
    setFollowing(true);
    shown = [];
    h.scroller = null;
});
afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
});

describe("AgentHistoryView setup and loading", () => {
    it("sets itself up in this order, reads the newest page, and tears down in this order", async () => {
        const r = mount();
        log("> mounted, before the reads settle");
        await flush();
        state(r.container, "loaded");
        log(`document props: ${Object.keys(h.doc).sort().join(",")}`);
        log(`navigator props: ${Object.keys(h.nav).sort().join(",")}`);
        log(`navigator: hasNewer=${h.nav.hasNewer()} hasOlder=${h.nav.hasOlder()} mode=${h.nav.mode()} archive=${h.nav.archiveZone()}`);
        log(`layout key: ${h.doc.blockId}`);
        r.unmount();
        await expect(out()).toMatchFileSnapshot(snap("setup"));
    });

    it("says so when there is nothing recorded, and when the read fails", async () => {
        disk.lines = [];
        let r = mount();
        await flush();
        state(r.container, "empty");
        r.unmount();
        T.trace.push("> a failing count");
        disk.lines = [user("q1")];
        fail.count = true;
        r = mount();
        await flush();
        state(r.container, "count failed");
        r.unmount();
        T.trace.push("> a failing range");
        fail.count = false;
        fail.range = true;
        r = mount();
        await flush();
        state(r.container, "range failed");
        await expect(out()).toMatchFileSnapshot(snap("empty-and-errors"));
    });
});

describe("AgentHistoryView while hidden and when a hole appears", () => {
    it("holds what arrives while the tab is hidden and catches up when it is shown", async () => {
        const r = mount();
        await flush();
        log("> hidden, a record arrives");
        setVis("dormant");
        await flush();
        write([user("q2")]);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        state(r.container, "hidden");
        log("> shown");
        setVis("active");
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        state(r.container, "shown");
        log("> hidden with a publish armed, then shown");
        write([text("a2")]);
        setVis("dormant");
        await flush();
        setVis("active");
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        state(r.container, "after the armed publish");
        log("> hidden, a truncate arrives, then shown");
        setVis("dormant");
        await flush();
        subject.next({ fileop: "truncate" });
        setVis("active");
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        state(r.container, "after the truncate");
        await expect(out()).toMatchFileSnapshot(snap("hidden"));
    });

    it("catches up by a count read, or reloads when the stream changed, the gap is too big or the read fails", async () => {
        const r = mount();
        await flush();
        const hide = async (label: string, change: () => void) => {
            log(`> ${label}`);
            T.trace.push("(hidden)");
            setVis("dormant");
            await flush();
            write([text(`during ${label}`)], false);
            subject.next({ fileop: "append", data64: "", pos: [] });
            change();
            setVis("active");
            await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
            state(r.container, label);
        };
        await hide("same stream", () => {});
        await hide("another generation", () => (disk.gen = "g2"));
        await hide("a gap too big to fill", () => disk.lines.push(...Array.from({ length: 700 }, (_, i) => text(`gap ${i}`))));
        await hide("a failing count", () => (fail.count = true));
        await expect(out()).toMatchFileSnapshot(snap("catch-up"));
    });

    it("recovers when a record can't be parsed, shows a jump button when the reader is reading older turns, and reloads at the bottom", async () => {
        const r = mount();
        await flush();
        log("> a record the parser rejects, following the bottom");
        fail.feed = true;
        write([text("bad")]);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        fail.feed = false;
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        state(r.container, "after the first reload, the parser still failing");
        log("> the same, reading older turns");
        setFollowing(false);
        fail.feed = true;
        write([text("bad again")]);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        fail.feed = false;
        state(r.container, "stale");
        log("> a record while stale is not shown");
        write([text("while stale")]);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        state(r.container, "still stale");
        log("> the reader scrolls back to the bottom");
        setFollowing(true);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        state(r.container, "reloaded at the bottom");
        log("> stale again, then Jump to latest");
        setFollowing(false);
        fail.feed = true;
        write([text("bad third")]);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        fail.feed = false;
        setFollowing(true);
        await flush();
        setFollowing(false);
        fail.feed = true;
        write([text("bad fourth")]);
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        fail.feed = false;
        (r.container.querySelector(".agent-history-jump-latest") as HTMLElement)?.click();
        await flush();
        state(r.container, "after Jump to latest");
        log("> the poll's hooks reach the same recovery");
        h.poll.recover();
        h.poll.reloadIfSkipped();
        log(`poll hidden()=${h.poll.hidden()} cursor=${h.poll.cursor() ? "set" : "null"}`);
        await expect(out()).toMatchFileSnapshot(snap("recover"));
    });
});

describe("AgentHistoryView jumps, archives and requested positions", () => {
    const longDisk = () => (disk.lines = Array.from({ length: HISTORY_PAGE * 2 + 50 }, (_, i) => (i === 7 ? user("early") : text(`line ${i}`))));

    it("jumps to a line, pages forward, opens an archive, and goes back to the latest", async () => {
        longDisk();
        const r = mount();
        await flush();
        state(r.container, "newest page");
        log("> jump to line 4000");
        h.nav.onJump(4000);
        await flush();
        state(r.container, "jumped");
        log(`mode=${h.nav.mode()} hasNewer=${h.nav.hasNewer()} hasOlder=${h.nav.hasOlder()}`);
        log("> show newer");
        h.nav.onShowNewer();
        await flush();
        state(r.container, "newer");
        h.nav.onShowNewer();
        await flush();
        log("> open an archive, then jump inside it");
        h.nav.onOpenArchive("agent:a1:archive:1");
        await flush();
        state(r.container, "archive");
        log(`mode=${h.nav.mode()} archive=${h.nav.archiveZone()}`);
        h.nav.onJump(1);
        await flush();
        log(`mode after a jump in an archive=${h.nav.mode()}`);
        log("> a failing archive read");
        fail.range = true;
        h.nav.onOpenArchive("agent:a1:archive:1");
        await flush();
        state(r.container, "archive failed");
        fail.range = false;
        log("> the archive's close, with no zone");
        h.nav.onOpenArchive(null);
        await flush();
        state(r.container, "latest again");
        log(`mode=${h.nav.mode()} archive=${h.nav.archiveZone()}`);
        log("> Latest");
        h.nav.onJump(100);
        await flush();
        h.nav.onLatest();
        await flush();
        log(`mode=${h.nav.mode()}`);
        await expect(out()).toMatchFileSnapshot(snap("modes"));
    });

    it("opens at a requested position, once per request, and asks for it to be cleared", async () => {
        longDisk();
        const [at, setAt] = createSignal<unknown>(undefined);
        const applied = vi.fn(() => log("onStartAtApplied()"));
        const r = mount({ startAt: at, onStartAtApplied: applied });
        await flush();
        log("> a byte in the current conversation");
        setAt({ at_byte: 123, n: 1 });
        await flush();
        state(r.container, "located");
        log(`mode=${h.nav.mode()}`);
        log("> the same request again, then a time in an archive");
        setAt({ at_byte: 123, n: 1 });
        await flush();
        setAt({ at_ms: 5000, zone: "agent:a1:archive:1", n: 2 });
        await flush();
        state(r.container, "archive located");
        log(`mode=${h.nav.mode()} archive=${h.nav.archiveZone()}`);
        log("> a zone that is the current one, then a failing locate");
        setAt({ at_ms: 6000, zone: "g:agent:a1:current", n: 3 });
        await flush();
        fail.locate = true;
        setAt({ at_ms: 7000, n: 4 });
        await flush();
        state(r.container, "locate failed");
        log("> not a position");
        setAt({ nothing: true });
        await flush();
        await expect(out()).toMatchFileSnapshot(snap("start-at"));
    });
});

describe("AgentHistoryView paging", () => {
    const longDisk = () => (disk.lines = Array.from({ length: HISTORY_PAGE * 3 + 50 }, (_, i) => (i % 40 === 0 ? user(`u${i}`) : text(`line ${i}`))));
    const geometry = (el: HTMLElement, g: { clientHeight: number; scrollHeight: number }, top = 0) => {
        let scrollTop = top;
        Object.defineProperty(el, "clientHeight", { configurable: true, get: () => g.clientHeight });
        Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => g.scrollHeight });
        Object.defineProperty(el, "scrollTop", { configurable: true, get: () => scrollTop, set: (v: number) => (log(`scrollTop = ${v}`), (scrollTop = v)) });
    };

    it("loads older pages from the list and from Show older, keeping the reader's place, and ignores a request with nothing older", async () => {
        longDisk();
        const r = mount();
        await flush();
        state(r.container, "newest page");
        log(`hasOlder=${h.nav.hasOlder()}`);
        log("> a read that fails: the ask is dropped quietly and can be made again");
        fail.range = true;
        await h.doc.onLoadOlder();
        fail.range = false;
        log(`after the failed read: hasOlder=${h.nav.hasOlder()} loadingOlder=${h.doc.loadingOlder()}`);
        geometry(h.scroller!, { clientHeight: 400, scrollHeight: 5000 }, 1200);
        log("> the list asks for older");
        await h.doc.onLoadOlder();
        log(`loadingOlder=${h.doc.loadingOlder()}`);
        state(r.container, "older");
        log("> two asks at once");
        const a = h.doc.onLoadOlder();
        const b = h.doc.onLoadOlder();
        await Promise.all([a, b]);
        log("> Show older");
        h.nav.onShowOlder();
        await flush();
        geometry(h.scroller!, { clientHeight: 400, scrollHeight: 9000 }, 100);
        log(`hasOlder=${h.nav.hasOlder()}`);
        await h.doc.onLoadOlder();
        await expect(out()).toMatchFileSnapshot(snap("older"));
    });

    it("scrolls up past the top to load older, fills a pane that is too short, and stops filling", async () => {
        longDisk();
        const r = mount();
        await flush();
        const body = r.container.querySelector(".agent-history-body") as HTMLElement;
        geometry(h.scroller!, { clientHeight: 400, scrollHeight: 5000 }, 0);
        log("> wheel up at the top");
        fireEvent.wheel(body, { deltaY: -50 });
        await flush(100);
        log("> wheel down, wheel up away from the top");
        fireEvent.wheel(body, { deltaY: 50 });
        geometry(h.scroller!, { clientHeight: 400, scrollHeight: 5000 }, 300);
        fireEvent.wheel(body, { deltaY: -50 });
        await flush(100);
        log("> a short page after a jump fills with older pages");
        geometry(h.scroller!, { clientHeight: 400, scrollHeight: 300 });
        h.nav.onJump(HISTORY_PAGE * 3);
        await flush(500);
        log(`reads: ${T.trace.filter((l) => l.startsWith("range(")).length}`);
        log("> a pane with no height, and one that is tall enough");
        geometry(h.scroller!, { clientHeight: 0, scrollHeight: 0 });
        h.nav.onJump(HISTORY_PAGE * 3 - 10);
        await flush(500);
        await expect(out()).toMatchFileSnapshot(snap("fill"));
    });
});

describe("AgentHistoryView races", () => {
    const longDisk = () => (disk.lines = Array.from({ length: HISTORY_PAGE * 2 + 50 }, (_, i) => (i % 50 === 0 ? user(`u${i}`) : text(`line ${i}`))));

    it("drops the answer of a read a newer one has replaced, and one that comes after the view is gone", async () => {
        longDisk();
        const r = mount({ zoomFactor: () => 1.5 });
        await flush();
        log(`zoomFactor passed: ${h.doc.zoomFactor()}`);
        log("> a reload started while another waits for its count");
        gate.hold.count = true;
        h.nav.onLatest();
        await flush();
        h.nav.onLatest();
        await flush();
        gate.hold.count = false;
        await release("count");
        await flush();
        state(r.container, "after the two reloads");
        log("> a reload replaced while its range is read, then the first fails");
        gate.hold.range = true;
        h.nav.onLatest();
        await flush();
        h.nav.onJump(2000);
        await flush();
        fail.range = true;
        await release("range");
        gate.hold.range = false;
        fail.range = false;
        await flush();
        state(r.container, "after the replaced range");
        log("> a count that fails after the load was replaced");
        gate.hold.count = true;
        h.nav.onJump(1500);
        await flush();
        fail.count = true;
        h.nav.onJump(1600);
        await flush();
        await release("count");
        fail.count = false;
        gate.hold.count = false;
        await release("count");
        await flush();
        state(r.container, "after the failing count");
        log("> a jump replaced while its read is held");
        gate.hold.range = true;
        h.nav.onJump(3000);
        await flush();
        gate.hold.range = false;
        h.nav.onJump(3100);
        await flush();
        await release("range");
        await flush();
        state(r.container, "after the replaced jump");
        log("> Show newer replaced by a reload, and Show older by one");
        gate.hold.range = true;
        h.nav.onShowNewer();
        await flush();
        h.nav.onLatest();
        await flush();
        await release("range");
        gate.hold.range = false;
        await flush();
        gate.hold.range = true;
        h.nav.onShowOlder();
        await flush();
        h.nav.onLatest();
        await flush();
        gate.hold.range = false;
        await release("range");
        await flush();
        state(r.container, "after the replaced pages");
        log("> the poll's recover in a static view does nothing");
        h.nav.onJump(500);
        await flush();
        const before = T.trace.length;
        h.poll.recover();
        await flush();
        log(`reads after recover: ${T.trace.length - before}`);
        log("> a load that finishes while the tab is hidden");
        setVis("dormant");
        await flush();
        h.nav.onLatest();
        await flush();
        state(r.container, "loaded while hidden");
        setVis("active");
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        state(r.container, "shown again");
        log("> the view goes while a count is held, and while a catch-up count is held");
        gate.hold.count = true;
        h.nav.onLatest();
        await flush();
        r.unmount();
        await release("count");
        await flush();
        const r2 = mount();
        await flush();
        gate.hold.count = false;
        setVis("dormant");
        await flush();
        subject.next({ fileop: "append", data64: "", pos: [] });
        gate.hold.count = true;
        setVis("active");
        await flush();
        r2.unmount();
        await release("count");
        await flush();
        log("done");
        await expect(out()).toMatchFileSnapshot(snap("races"));
    });

    it("treats records from a new generation as a reason to reload, and a request that arrives after the view is gone as nothing", async () => {
        const [at, setAt] = createSignal<unknown>(undefined);
        const r = mount({ startAt: at, onStartAtApplied: () => log("onStartAtApplied()") });
        await flush();
        log("> an append from another generation");
        subject.next({
            fileop: "append",
            data64: btoa(text("from g2") + "\n"),
            pos: [{ stream: STREAM, gen: "g2", line: 0, lines: 1 }],
        });
        await flush(HISTORY_PUBLISH_MIN_INTERVAL_MS);
        state(r.container, "after the append");
        log("> the view goes, then a position is asked for");
        r.unmount();
        setAt({ at_byte: 5, n: 1 });
        await flush();
        await expect(out()).toMatchFileSnapshot(snap("generation"));
    });
});
