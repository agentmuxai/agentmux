// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentShellSubblock: the paths the main suite leaves open — what the
 * terminal is built with, the live font/scroll updates around `init()`, the
 * loading overlay's fade and its return on a repoint, the agent lock's expiry,
 * which exits count, and the stale-attach branches (unmount mid-attach, a
 * newer repoint overtaking a resync, a create or an init). Pinned before the
 * component was split into smaller functions.
 */

import { resolveContextMenuRegion } from "@/app/block/context-menu-region";
import { cleanup, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentShellSubblock } from "./AgentShellSubblock";

type Deferred<T = void> = { promise: Promise<T>; resolve: (v: T) => void; reject: (e: unknown) => void };
const deferred = <T = void,>(): Deferred<T> => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
};

const h = vi.hoisted(() => ({
    blockSignals: new Map<
        string,
        [() => { value: any; loading: boolean }, (v: { value: any; loading: boolean }) => void]
    >(),
    mpsHandlers: new Map<string, Array<(event: any) => void>>(),
    mpsPersisted: new Map<string, Record<string, unknown>>(),
    settings: { set: (_bag: Record<string, unknown>) => {} },
    motion: { divisor: 1, reduce: false },
    resyncHold: new Map<string, { promise: Promise<void>; resolve: () => void }>(),
    resyncFail: new Set<string>(),
    createQueue: [] as Array<() => Promise<string>>,
    initGates: [] as Array<Promise<void>>,
    wraps: [] as any[],
    observers: [] as any[],
    senders: [] as Array<{ id: string; sent: string[] }>,
    zoomWheel: [] as Array<[() => HTMLElement | undefined, () => string | null | undefined, number | undefined]>,
    selection: { text: "" },
}));

vi.mock("@/app/store/mps", () => {
    const muxEventSubscribe = (opts: { eventType: string; scope: string; handler: (event: any) => void }) => {
        const key = `${opts.eventType}|${opts.scope}`;
        h.mpsHandlers.set(key, [...(h.mpsHandlers.get(key) ?? []), opts.handler]);
        const persisted = h.mpsPersisted.get(key);
        if (persisted !== undefined) opts.handler({ data: persisted });
        return () => {
            h.mpsHandlers.set(
                key,
                (h.mpsHandlers.get(key) ?? []).filter((x) => x !== opts.handler)
            );
        };
    };
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        muxEventSubscribe({
            eventType,
            scope,
            handler: (e: any) => {
                if (e?.data != null) handler(e.data, e);
            },
        } as any);
    return { muxEventSubscribe, onMuxEvent };
});

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ControllerResyncCommand: vi.fn((_c: unknown, p: { blockid: string }) => {
            if (h.resyncFail.has(p.blockid)) return Promise.reject(new Error("not found"));
            return h.resyncHold.get(p.blockid)?.promise ?? Promise.resolve();
        }),
        CreateSubBlockCommand: vi.fn(() =>
            (h.createQueue.shift() ?? (() => Promise.resolve("block:new-sub-block-id")))()
        ),
        DeleteSubBlockCommand: vi.fn(() => Promise.resolve()),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/util/clipboard", () => ({
    readText: vi.fn(() => Promise.resolve("pasted text")),
    writeText: vi.fn(() => Promise.resolve()),
}));
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: vi.fn() }));

vi.mock("@/app/store/global", async () => {
    const { createSignal: sig } = await import("solid-js");
    const [bag, setBag] = sig<Record<string, unknown>>({});
    h.settings.set = (b) => setBag({ ...b });
    const signalFor = (oref: string) => {
        let s = h.blockSignals.get(oref);
        if (!s) {
            s = sig<{ value: any; loading: boolean }>({ value: null, loading: true });
            h.blockSignals.set(oref, s);
        }
        return s;
    };
    const MOS = {
        makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
        getMuxObjectAtom: (oref: string) => {
            const [get] = signalFor(oref);
            return () => get().value;
        },
        getMuxObjectLoadingAtom: (oref: string) => {
            const [get] = signalFor(oref);
            return () => (get().loading ? null : get().loading);
        },
    };
    return {
        MOS,
        staticTabId: () => "tab-1",
        getSettingsPrefixAtom: (prefix: string) => () => (prefix === "term" ? bag() : {}),
    };
});

vi.mock("@/app/motion/motion", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/motion/motion")>()),
    scaledMs: (ms: number) => ms / h.motion.divisor,
    shouldReduceMotion: () => h.motion.reduce,
}));

vi.mock("@/app/store/zoom", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/zoom")>()),
    usePaneZoomWheel: (el: any, id: any, base: any) => void h.zoomWheel.push([el, id, base]),
}));

vi.mock("@/app/view/term/block-input-sender", () => ({
    BlockInputSender: class {
        sent: string[] = [];
        constructor(public id: string) {
            h.senders.push(this as any);
        }
        send(data: string) {
            this.sent.push(data);
        }
    },
}));

vi.mock("@/app/view/term/termwrap", () => ({
    TermWrap: class {
        terminal: any;
        handleResize = vi.fn();
        handleResizeLive = vi.fn();
        dispose = vi.fn();
        constructor(
            public id: string,
            public container: HTMLElement,
            public options: Record<string, unknown>,
            public mux: { sendDataHandler: (d: string) => void; keydownHandler?: (e: KeyboardEvent) => boolean }
        ) {
            this.terminal = {
                options: { fontSize: options.fontSize, scrollback: options.scrollback, scrollSensitivity: undefined },
                write: vi.fn(),
                paste: vi.fn(),
                getSelection: () => h.selection.text,
            };
            h.wraps.push(this);
        }
        async init() {
            const gate = h.initGates.shift();
            if (gate) await gate;
        }
    },
}));

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const settle = async () => {
    for (let i = 0; i < 10; i++) await sleep(0);
};

const wraps = () => h.wraps;
const overlay = () => document.querySelector(".agent-shell-loading-overlay");
const errorText = () => document.querySelector(".agent-shell-subblock-error")?.textContent ?? null;
const badge = () => document.querySelector(".agent-shell-agentlock-badge");

/** A block whose meta is already there, so the seed wait ends at once. */
function seedNow(id: string, meta: Record<string, unknown> = {}) {
    const [, set] = signalOf(`block:${id}`);
    set({ value: { meta }, loading: false });
}
/** A block whose fetch is still in flight. */
function seedLater(id: string) {
    signalOf(`block:${id}`);
}
function settleSeed(id: string, meta: Record<string, unknown> = {}) {
    seedNow(id, meta);
}
function signalOf(oref: string) {
    let s = h.blockSignals.get(oref);
    if (!s) {
        s = createSignal<{ value: any; loading: boolean }>({ value: null, loading: true });
        h.blockSignals.set(oref, s as never);
    }
    return s as [() => { value: any; loading: boolean }, (v: { value: any; loading: boolean }) => void];
}
function holdResync(id: string) {
    const d = deferred();
    h.resyncHold.set(id, { promise: d.promise, resolve: d.resolve });
    return () => {
        d.resolve();
        h.resyncHold.delete(id);
    };
}
function holdNextInit() {
    const d = deferred();
    h.initGates.push(d.promise);
    return d.resolve;
}
function holdNextCreate() {
    const d = deferred<string>();
    h.createQueue.push(() => d.promise);
    return d;
}
function emitStatus(blockId: string, data: Record<string, unknown>) {
    for (const fn of h.mpsHandlers.get(`controllerstatus|block:${blockId}`) ?? []) fn({ data });
}

function mountShell(over: { existingSubBlockId?: string } = {}) {
    const [existing, setExisting] = createSignal<string | undefined>(over.existingSubBlockId);
    const props = {
        onSubBlockCreated: vi.fn(),
        onTermReady: vi.fn(),
        onTermDispose: vi.fn(),
        onShellExited: vi.fn(),
    };
    const r = render(() => (
        <AgentShellSubblock parentBlockId="parent-1" cwd="/work" existingSubBlockId={existing()} {...props} />
    ));
    return {
        ...props,
        setExisting,
        unmount: r.unmount,
        surface: r.container.querySelector(".agent-shell-subblock") as HTMLElement,
    };
}

beforeEach(async () => {
    h.blockSignals.clear();
    h.mpsHandlers.clear();
    h.mpsPersisted.clear();
    h.resyncHold.clear();
    h.resyncFail.clear();
    h.createQueue.length = 0;
    h.initGates.length = 0;
    h.wraps.length = 0;
    h.observers.length = 0;
    h.senders.length = 0;
    h.zoomWheel.length = 0;
    h.selection.text = "";
    h.motion.divisor = 1;
    h.motion.reduce = false;
    h.settings.set({});
    (globalThis as any).ResizeObserver = class {
        observe = vi.fn();
        disconnect = vi.fn();
        constructor(public callback: () => void) {
            h.observers.push(this);
        }
    };
    const rpc = (await import("@/app/store/rpc-api")).RpcApi as any;
    rpc.ControllerResyncCommand.mockClear();
    rpc.CreateSubBlockCommand.mockClear();
    rpc.DeleteSubBlockCommand.mockClear();
});

afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe("AgentShellSubblock — the calls it makes", () => {
    it("checks an existing sub-block with a resync that doesn't revive it, in this tab", async () => {
        const { RpcApi } = await import("@/app/store/rpc-api");
        seedNow("A");
        mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(wraps()).toHaveLength(1));
        expect(RpcApi.ControllerResyncCommand).toHaveBeenCalledWith(
            {},
            { tabid: "tab-1", blockid: "A", forcerestart: false, norespawn: true }
        );
    });

    it("creates a new shell under the pane, in the pane's folder, and tells the parent", async () => {
        const { RpcApi } = await import("@/app/store/rpc-api");
        const m = mountShell();
        await waitFor(() => expect(wraps()).toHaveLength(1));
        expect(RpcApi.CreateSubBlockCommand).toHaveBeenCalledWith(
            {},
            { parentblockid: "parent-1", blockdef: { meta: { view: "term", controller: "shell", "cmd:cwd": "/work" } } }
        );
        expect(m.onSubBlockCreated).toHaveBeenCalledTimes(1);
        expect(m.onSubBlockCreated).toHaveBeenCalledWith("new-sub-block-id");
        expect(wraps()[0].id).toBe("new-sub-block-id");
        expect(h.senders[0].id).toBe("new-sub-block-id");
    });

    it("zooms by the wheel on its own surface, by its own sub-block, from 13", async () => {
        const m = mountShell();
        await waitFor(() => expect(wraps()).toHaveLength(1));
        expect(h.zoomWheel).toHaveLength(1);
        const [el, id, base] = h.zoomWheel[0];
        expect(el()).toBe(m.surface);
        expect(id()).toBe("new-sub-block-id");
        expect(base).toBe(13);
    });
});

describe("AgentShellSubblock — what the terminal is built with", () => {
    it("builds it with the drawer's font, rendering and key options", async () => {
        const m = mountShell();
        await waitFor(() => expect(wraps()).toHaveLength(1));
        const w = wraps()[0];
        expect(w.container).toBe(m.surface);
        expect(w.options).toMatchObject({
            fontSize: 13,
            fontFamily: "Hack",
            allowTransparency: false,
            scrollback: 2000,
            allowProposedApi: true,
        });
        expect(w.mux).toMatchObject({ useWebGl: true });
        expect(w.mux.keydownHandler).toBeTypeOf("function");
    });

    it("hands the parent a writer that starts its text on a fresh line and ends it", async () => {
        const m = mountShell();
        await waitFor(() => expect(m.onTermReady).toHaveBeenCalledTimes(1));
        const write = m.onTermReady.mock.calls[0][0] as (t: string) => void;
        write("hello");
        expect(wraps()[0].terminal.write).toHaveBeenCalledWith("\r\nhello\r\n");
    });

    it("refits on each resize of its surface, with the live resize", async () => {
        const m = mountShell();
        await waitFor(() => expect(h.observers).toHaveLength(1));
        expect(h.observers[0].observe).toHaveBeenCalledWith(m.surface);
        await settle();
        const refits = wraps()[0].handleResize.mock.calls.length;
        h.observers[0].callback();
        expect(wraps()[0].handleResizeLive).toHaveBeenCalledTimes(1);
        expect(wraps()[0].handleResize).toHaveBeenCalledTimes(refits);
    });

    it("sends typed input to its own sub-block's sender", async () => {
        mountShell();
        await waitFor(() => expect(wraps()).toHaveLength(1));
        wraps()[0].mux.sendDataHandler("ls\r");
        expect(h.senders[0].sent).toEqual(["ls\r"]);
    });
});

describe("AgentShellSubblock — waiting for the seed", () => {
    it("waits for a slow seed fetch rather than giving up after a moment", async () => {
        seedLater("slow");
        mountShell({ existingSubBlockId: "slow" });
        await sleep(150);
        expect(wraps()).toHaveLength(0);
        settleSeed("slow", { "term:zoom": 2 });
        await waitFor(() => expect(wraps()).toHaveLength(1));
        expect(wraps()[0].options.fontSize).toBe(26);
    });
});

describe("AgentShellSubblock — live updates that land around init()", () => {
    it("applies a zoom that changed while the terminal was still starting, and refits", async () => {
        seedNow("A", { "term:zoom": 1 });
        const release = holdNextInit();
        mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(wraps()).toHaveLength(1));
        expect(wraps()[0].options.fontSize).toBe(13);

        seedNow("A", { "term:zoom": 2 });
        release();
        await waitFor(() => expect(wraps()[0].terminal.options.fontSize).toBe(26));
        expect(wraps()[0].handleResize).toHaveBeenCalledTimes(1);
    });

    it("applies scroll sensitivity and scrollback that changed while it was starting", async () => {
        const release = holdNextInit();
        mountShell();
        await waitFor(() => expect(wraps()).toHaveLength(1));
        h.settings.set({ "term:scrollsensitivity": 3, "term:scrollback": 5000 });
        release();
        await waitFor(() => expect(wraps()[0].terminal.options.scrollSensitivity).toBe(3));
        expect(wraps()[0].terminal.options.scrollback).toBe(5000);
    });

    it("still applies a change that lands while a REPOINTED terminal starts", async () => {
        seedNow("A", { "term:zoom": 1 });
        seedNow("B", { "term:zoom": 1 });
        const m = mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(wraps()).toHaveLength(1));

        const release = holdNextInit();
        m.setExisting("B");
        await waitFor(() => expect(wraps()).toHaveLength(2));
        seedNow("B", { "term:zoom": 2 });
        release();
        await waitFor(() => expect(wraps()[1].terminal.options.fontSize).toBe(26));
    });
});

describe("AgentShellSubblock — the loading overlay", () => {
    it("is gone a fade after the terminal is built, in the scaled time", async () => {
        h.motion.divisor = 8; // 20 ms
        mountShell();
        await waitFor(() => expect(wraps()).toHaveLength(1));
        await sleep(100);
        expect(overlay()).toBeNull();
    });

    it("is gone at once when the motion setting reduces transitions", async () => {
        h.motion.reduce = true;
        mountShell();
        await waitFor(() => expect(wraps()).toHaveLength(1));
        await settle();
        expect(overlay()).toBeNull();
    });

    it("comes back, not fading, and is not hidden by the old fade, when the shell is repointed", async () => {
        const m = mountShell();
        await waitFor(() => expect(wraps()).toHaveLength(1));
        holdResync("B");
        m.setExisting("B");
        await sleep(400); // longer than the 160 ms fade the old attach had running
        expect(overlay()).not.toBeNull();
        expect(overlay()!.classList.contains("is-fading")).toBe(false);
    });

    it("comes back after it had finished fading away, when the shell is repointed", async () => {
        h.motion.divisor = 8;
        const m = mountShell();
        await waitFor(() => expect(wraps()).toHaveLength(1));
        await sleep(100);
        expect(overlay()).toBeNull();
        holdResync("B");
        m.setExisting("B");
        await settle();
        expect(overlay()).not.toBeNull();
    });

    it("leaves no timer behind on unmount, neither the lock tick nor a fade", async () => {
        vi.useFakeTimers();
        const m = mountShell();
        for (let i = 0; i < 10; i++) await vi.advanceTimersByTimeAsync(0);
        expect(wraps()).toHaveLength(1);
        expect(vi.getTimerCount()).toBeGreaterThan(0);
        m.unmount();
        expect(vi.getTimerCount()).toBe(0);
    });
});

describe("AgentShellSubblock — the agent lock", () => {
    it("lets the human type again once the lock lapses, with no meta update to say so", async () => {
        seedNow("A", { "term:agentlockuntil": Date.now() + 300 });
        mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(wraps()).toHaveLength(1));
        wraps()[0].mux.sendDataHandler("a");
        expect(h.senders[0].sent).toEqual([]);
        await sleep(1100);
        wraps()[0].mux.sendDataHandler("b");
        expect(h.senders[0].sent).toEqual(["b"]);
        expect(badge()).toBeNull();
    });

    it("says why the keys are dropped", async () => {
        seedNow("A", { "term:agentlockuntil": Date.now() + 60_000 });
        mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(badge()).not.toBeNull());
        expect(badge()).toHaveTextContent("Agent is using this shell");
    });

    it("ignores a lock time that isn't a number", async () => {
        seedNow("A", { "term:agentlockuntil": "99999999999999" });
        mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(wraps()).toHaveLength(1));
        wraps()[0].mux.sendDataHandler("a");
        expect(h.senders[0].sent).toEqual(["a"]);
        expect(badge()).toBeNull();
    });

    it("is not locked at the very moment the lease runs out", async () => {
        vi.useFakeTimers({ toFake: ["Date"] });
        vi.setSystemTime(1_000_000);
        seedNow("A", { "term:agentlockuntil": 1_000_000 });
        mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(wraps()).toHaveLength(1));
        wraps()[0].mux.sendDataHandler("a");
        expect(h.senders[0].sent).toEqual(["a"]);
    });
});

describe("AgentShellSubblock — which exits count", () => {
    it("does not listen for an exit before it has a sub-block", async () => {
        const create = holdNextCreate();
        const m = mountShell();
        await settle();
        expect([...h.mpsHandlers.keys()].filter((k) => k.startsWith("controllerstatus"))).toEqual([]);
        create.resolve("block:fresh");
        await waitFor(() => expect(h.mpsHandlers.get("controllerstatus|block:fresh")?.length).toBe(1));
        expect(m.onShellExited).not.toHaveBeenCalled();
    });

    it("counts a done after a running only, and not other statuses", async () => {
        seedNow("A");
        const m = mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(h.mpsHandlers.get("controllerstatus|block:A")?.length).toBe(1));
        emitStatus("A", { shellprocstatus: "running" });
        emitStatus("A", { shellprocstatus: "init", shellprocexitcode: 0 });
        expect(m.onShellExited).not.toHaveBeenCalled();
        emitStatus("A", { shellprocstatus: "done", shellprocexitcode: 0 });
        expect(m.onShellExited).toHaveBeenCalledTimes(1);
    });

    it("deletes a block whose replayed exit came without a code, as a clean exit", async () => {
        const { RpcApi } = await import("@/app/store/rpc-api");
        h.resyncFail.add("A");
        h.mpsPersisted.set("controllerstatus|block:A", { shellprocstatus: "done" });
        seedNow("A");
        mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(RpcApi.DeleteSubBlockCommand).toHaveBeenCalledWith({}, { blockid: "A" }));
    });
});

describe("AgentShellSubblock — repointing", () => {
    it("attaches again to the first sub-block when the parent repoints back to it", async () => {
        seedNow("A");
        seedNow("B");
        const m = mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(wraps()).toHaveLength(1));
        m.setExisting("B");
        await waitFor(() => expect(wraps()).toHaveLength(2));
        expect(wraps()[0].dispose).toHaveBeenCalledTimes(1);
        m.setExisting("A");
        await waitFor(() => expect(wraps()).toHaveLength(3));
        expect(wraps()[2].id).toBe("A");
    });

    it("does nothing when the parent clears the pointer", async () => {
        seedNow("A");
        const m = mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(wraps()).toHaveLength(1));
        m.setExisting(undefined);
        await settle();
        expect(wraps()).toHaveLength(1);
        expect(wraps()[0].dispose).not.toHaveBeenCalled();
    });

    it("lets go of the old terminal at once: its writer is dead, its observer disconnected once", async () => {
        seedNow("A");
        const m = mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(m.onTermReady).toHaveBeenCalledTimes(1));
        const write = m.onTermReady.mock.calls[0][0] as (t: string) => void;

        holdResync("B");
        m.setExisting("B");
        await settle();
        write("late");
        expect(wraps()[0].terminal.write).not.toHaveBeenCalled();
        expect(h.observers[0].disconnect).toHaveBeenCalledTimes(1);

        m.unmount();
        expect(h.observers[0].disconnect).toHaveBeenCalledTimes(1);
    });

    it("clears the last start's error", async () => {
        const create = holdNextCreate();
        const m = mountShell();
        create.reject(new Error("boom"));
        await waitFor(() => expect(errorText()).toBe("Shell failed to start: boom"));
        seedNow("B");
        m.setExisting("B");
        await waitFor(() => expect(wraps()).toHaveLength(1));
        expect(errorText()).toBeNull();
    });
});

describe("AgentShellSubblock — an attach that is overtaken or abandoned", () => {
    it("builds nothing when it is unmounted while checking the old sub-block", async () => {
        seedNow("A");
        const release = holdResync("A");
        const m = mountShell({ existingSubBlockId: "A" });
        await settle();
        m.unmount();
        release();
        await settle();
        expect(wraps()).toHaveLength(0);
        expect(m.onSubBlockCreated).not.toHaveBeenCalled();
    });

    it("does not point itself back at an older sub-block whose resync finishes late", async () => {
        seedNow("A");
        seedNow("B");
        const releaseA = holdResync("A");
        const m = mountShell({ existingSubBlockId: "A" });
        await settle();
        m.setExisting("B");
        await waitFor(() => expect(wraps()).toHaveLength(1));
        expect(h.zoomWheel[0][1]()).toBe("B");
        releaseA();
        await settle();
        expect(h.zoomWheel[0][1]()).toBe("B");
        expect(wraps()).toHaveLength(1);
        expect(wraps()[0].id).toBe("B");
    });

    it("deletes a sub-block it created too late and doesn't adopt or announce it", async () => {
        const { RpcApi } = await import("@/app/store/rpc-api");
        seedNow("B");
        const create = holdNextCreate();
        const m = mountShell();
        await settle();
        m.setExisting("B");
        await waitFor(() => expect(wraps()).toHaveLength(1));
        create.resolve("block:late");
        await settle();
        expect(RpcApi.DeleteSubBlockCommand).toHaveBeenCalledWith({}, { blockid: "late" });
        expect(m.onSubBlockCreated).not.toHaveBeenCalledWith("late");
        expect(h.zoomWheel[0][1]()).toBe("B");
        expect(wraps()).toHaveLength(1);
    });

    it("doesn't seed the overlay away for an attach that was overtaken while it waited on the seed", async () => {
        seedLater("A");
        seedNow("B");
        const m = mountShell({ existingSubBlockId: "A" });
        await settle();
        holdResync("B");
        m.setExisting("B");
        await settle();
        settleSeed("A");
        await sleep(100);
        expect(wraps()).toHaveLength(0);
        expect(overlay()).not.toBeNull();
        expect(overlay()!.classList.contains("is-fading")).toBe(false);
    });

    it("disposes a terminal whose init finished after it was overtaken, and never announces it", async () => {
        seedNow("A");
        seedNow("B");
        const releaseA = holdNextInit();
        const m = mountShell({ existingSubBlockId: "A" });
        await waitFor(() => expect(wraps()).toHaveLength(1));
        m.setExisting("B");
        await waitFor(() => expect(wraps()).toHaveLength(2));
        await waitFor(() => expect(m.onTermReady).toHaveBeenCalledTimes(1));

        releaseA();
        await settle();
        expect(wraps()[0].dispose).toHaveBeenCalledTimes(1);
        expect(m.onTermReady).toHaveBeenCalledTimes(1);
        expect(h.observers).toHaveLength(1);
    });

    it("says nothing of a failure that was overtaken, but logs it", async () => {
        const log = vi.spyOn(console, "error").mockImplementation(() => {});
        seedNow("B");
        const create = holdNextCreate();
        const m = mountShell();
        await settle();
        m.setExisting("B");
        await waitFor(() => expect(wraps()).toHaveLength(1));
        create.reject(new Error("too late"));
        await settle();
        expect(errorText()).toBeNull();
        expect(log).toHaveBeenCalledWith("AgentShellSubblock: failed to start shell:", expect.any(Error));
    });

    it("shows a failure's message, or its text when it isn't an Error", async () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        h.createQueue.push(() => Promise.reject(new Error("boom")));
        mountShell();
        await waitFor(() => expect(errorText()).toBe("Shell failed to start: boom"));
        cleanup();

        h.createQueue.push(() => Promise.reject("weird"));
        mountShell();
        await waitFor(() => expect(errorText()).toBe("Shell failed to start: weird"));
    });
});

describe("AgentShellSubblock — unmount", () => {
    it("disposes the terminal, disconnects the observer and tells the parent", async () => {
        const m = mountShell();
        await waitFor(() => expect(h.observers).toHaveLength(1));
        m.unmount();
        expect(wraps()[0].dispose).toHaveBeenCalledTimes(1);
        expect(h.observers[0].disconnect).toHaveBeenCalledTimes(1);
        expect(m.onTermDispose).toHaveBeenCalledTimes(1);
    });
});

describe("AgentShellSubblock — the right-click menu's actions", () => {
    it("Copy writes the terminal's selection to the clipboard", async () => {
        const { writeText } = await import("@/util/clipboard");
        const m = mountShell();
        await waitFor(() => expect(wraps()).toHaveLength(1));
        h.selection.text = "npm test";
        const [copy] = resolveContextMenuRegion(m.surface)!.items!();
        copy.click!();
        expect(writeText).toHaveBeenCalledWith("npm test");
    });

    it("Paste says so when the clipboard is too large for the shell", async () => {
        const { readText } = await import("@/util/clipboard");
        const { pushNotification } = await import("@/app/store/flash-notifications");
        (readText as any).mockResolvedValueOnce("x".repeat(1024 * 1024 + 1));
        const m = mountShell();
        await waitFor(() => expect(wraps()).toHaveLength(1));
        const [, paste] = resolveContextMenuRegion(m.surface)!.items!();
        paste.click!();
        await waitFor(() => expect(pushNotification).toHaveBeenCalled());
        expect((pushNotification as any).mock.calls[0][0]).toMatchObject({
            title: "Paste too large for the shell",
            type: "warning",
        });
        expect(wraps()[0].terminal.paste).not.toHaveBeenCalled();
    });
});
