// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What constructing a BrowserViewModel does, pinned as ordered logs: the diagnostics it writes, the
 * host events it subscribes to (in order) and what each does when it fires, the first page it opens,
 * and the value of every atom it derives from the pane's meta. A refactor of the constructor must
 * leave the committed snapshots as they are; regenerate with `vitest -u` only for a change meant to
 * behave differently.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const log = vi.hoisted(() => [] as string[]);
const regs = vi.hoisted(() => [] as { name: string; cb: (p: any) => void; unsub: () => void; resolve: () => void }[]);
const hold = vi.hoisted(() => ({ subscriptions: false }));

vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: (cmd: string, args: unknown) => {
        log.push(`invoke ${cmd} ${JSON.stringify(args)}`);
        return Promise.resolve();
    },
    listenEvent: (name: string, cb: (p: any) => void) => {
        log.push(`listen ${name}`);
        const unsub = () => log.push(`unsub ${name}`);
        return new Promise<() => void>((resolve) => {
            const reg = { name, cb, unsub, resolve: () => resolve(unsub) };
            regs.push(reg);
            if (!hold.subscriptions) reg.resolve();
        });
    },
}));
vi.mock("@/app/store/global", () => ({ refocusNode: vi.fn() }));
vi.mock("@/app/store/block-component-registry", async (importOriginal) => {
    const real = await importOriginal<typeof import("@/app/store/block-component-registry")>();
    return { ...real, refocusNode: (id: string) => log.push(`refocusNode ${id}`) };
});
vi.mock("@/app/store/dev-mode", () => ({ devModeOn: () => false }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetMetaCommand: vi.fn(() => Promise.resolve()) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mos", () => ({
    makeORef: (type: string, id: string) => `${type}:${id}`,
    getMuxObjectAtom: () => () => ({ meta: {} }),
    getObjectValue: () => ({}),
}));
vi.mock("@/layout/lib/layoutModelHooks", () => ({
    getLayoutModelForStaticTab: () => ({
        getNodeByBlockId: (blockId: string) => ({ id: `node-${blockId}`, data: { stack: [blockId] } }),
        closeNode: (nodeId: string) => {
            log.push(`closeNode ${nodeId}`);
            return Promise.resolve();
        },
    }),
    getLayoutModelForTabById: () => undefined,
}));
vi.mock("@/app/util/reveal-block", () => ({ findTabHolding: () => Promise.resolve(null) }));
vi.mock("@/layout/lib/layoutStack", () => ({
    closeBlockInStack: () => Promise.resolve(),
    effectiveStack: (d: { stack: string[] }) => d.stack,
}));

import { setFullConfigAtom } from "@/store/config-signals";
import { snapshot as bpSnapshot } from "@/app/store/browser-pane-state-store";
import { BrowserViewModel } from "./browser-model";
import { installCefWireHost } from "../../../test/cef-wire-host";

installCefWireHost();

const BLOCK = "ctor-block-1234";

/** A host context whose meta is a signal; `setMeta` is logged, and does not change the meta. */
function ctxFor(initial: Record<string, unknown>) {
    const [meta, setMeta] = createSignal<Record<string, unknown>>(initial);
    return {
        meta,
        setMetaSignal: setMeta,
        ctx: {
            blockId: BLOCK,
            meta,
            setMeta: (patch: Record<string, unknown>) => {
                log.push(`ctx.setMeta ${JSON.stringify(patch)}`);
                return Promise.resolve();
            },
            isFocused: () => false,
            visibility: () => "active" as const,
        },
    };
}

let disposers: (() => void)[] = [];
/** Builds a model inside its own root, so its effects are torn down with the test. */
function build(initial: Record<string, unknown> = {}) {
    const c = ctxFor(initial);
    let vm!: BrowserViewModel;
    disposers.push(createRoot((d) => {
        vm = new BrowserViewModel(c.ctx as never);
        return d;
    }));
    return { vm, ...c };
}

const sv = (v: unknown) => JSON.stringify(v, (_k, x) => (x === undefined ? "<undefined>" : x), 1);
function atoms(vm: BrowserViewModel): string {
    return sv({
        url: vm.urlAtom(), title: vm.titleAtom(), favicon: vm.faviconUrlAtom(), loading: vm.loadingAtom(),
        canGoBack: vm.canGoBackAtom(), canGoForward: vm.canGoForwardAtom(), error: vm.errorAtom(),
        viewName: vm.viewName(), viewNameIsPlaceholder: vm.viewNameIsPlaceholder(), viewFaviconUrl: vm.viewFaviconUrl(),
        showControls: vm.showControlsAtom(), attention: vm.attentionAtom(), bypass: vm.bypassAtom(), bypassEnded: vm.bypassEndedAtom(),
        actions: vm.actionsAtom(), paused: vm.pausedAtom(), driver: vm.driverAgentAtom(), lastDriver: vm.lastDriverAtom(),
        identity: vm.identityAtom(), sleep: vm.sleepAtom(), loadMs: vm.loadMsAtom(), keepAwake: vm.keepAwakeAtom(),
        allowedOrigins: vm.allowedOriginsAtom(), popupFrom: vm.popupFromAtom(), popupWindows: vm.popupWindowsAtom(),
        downloads: vm.downloadsAtom(), closed: vm.closed,
    });
}
const slot = () => {
    const s = bpSnapshot(BLOCK) as any;
    return sv(s && { closed: s.closed, activeTabId: s.activeTabId ? "<tab>" : s.activeTabId, tabs: s.tabs?.map((t: any) => ({ ...t, id: "<id>" })) });
};
const fire = (name: string, payload: unknown) => {
    const r = [...regs].reverse().find((x) => x.name === name);
    if (!r) throw new Error(`no subscription for ${name}`);
    r.cb(payload);
};
const settle = (ms = 0) => vi.advanceTimersByTimeAsync(ms);
const out = () => log.join("\n") + "\n";
const snap = (name: string) => `./__snapshots__/browser-model.ctor.${name}.txt`;

beforeEach(() => {
    log.length = 0;
    regs.length = 0;
    hold.subscriptions = false;
    disposers = [];
    setFullConfigAtom(null);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date"] });
    vi.setSystemTime(new Date("2026-11-03T12:00:00Z"));
    vi.spyOn(console, "log").mockImplementation((...a: unknown[]) => {
        const s = String(a[0]);
        if (s.startsWith("[browser-pane:diag]")) log.push(`diag ${s.replace(/vm=[a-z0-9]+/, "vm=<id>").replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, "<uuid>")}`);
    });
});
afterEach(() => {
    for (const d of disposers) d();
    vi.restoreAllMocks();
    vi.useRealTimers();
    setFullConfigAtom(null);
});

describe("construction", () => {
    const cases: [string, Record<string, unknown>, string | null][] = [
        ["no url: the default", {}, null],
        ["a bare host", { url: "example.org" }, null],
        ["a search phrase", { url: "how to bake bread" }, null],
        ["a blank url: the start page", { url: "   " }, "https://start.example"],
        ["about:blank", { url: "about:blank" }, null],
        ["a host error page's data: address", { url: "data:text/html,<h1>failed</h1>" }, null],
        ["an explicit url with a start page set: the url wins", { url: "https://given.example/x" }, "https://start.example"],
    ];

    it.each(cases)("%s", async (_name, meta, start) => {
        setFullConfigAtom(start ? ({ browserstartpage: start } as never) : null);
        const m = build(meta);
        await settle();
        log.push(`slot ${slot()}`);
        log.push(`atoms ${atoms(m.vm)}`);
        const file = _name.replace(/[^a-z0-9]+/gi, "-").toLowerCase();
        await expect(out()).toMatchFileSnapshot(snap(`construct-${file}`));
    });

    it("subscribes to the host's events in order, and unsubscribes them in order on dispose", async () => {
        const m = build();
        await settle();
        log.push("-- dispose");
        m.vm.dispose();
        log.push(`slot ${slot()}`);
        log.push(`atoms ${atoms(m.vm)}`);
        await expect(out()).toMatchFileSnapshot(snap("subscriptions"));
    });

    it("a subscription that resolves after dispose is dropped at once, and a later one is kept", async () => {
        hold.subscriptions = true;
        const m = build();
        log.push("-- subscriptions pending; dispose");
        m.vm.dispose();
        for (const r of regs) r.resolve();
        await settle();
        log.push("-- a second model whose subscriptions resolve while open");
        regs.length = 0;
        const m2 = build();
        for (const r of regs) r.resolve();
        await settle();
        log.push("-- dispose");
        m2.vm.dispose();
        await expect(out()).toMatchFileSnapshot(snap("subscriptions-late"));
    });
});

describe("the atoms the pane's meta feeds", () => {
    const full = {
        url: "https://a.example/",
        "browser:show_controls": false,
        "browser:attention": { id: "att-1", kind: "handoff", agent: "Korp" },
        "browser:actions": { count: 7, entries: [{ ts: 5, kind: "click" }, { ts: "x", kind: "bad" }, { ts: 6 }] },
        "browser:paused": true,
        "browser:owner_agent": "  Korp  ",
        "browser:last_driver": " Ada ",
        "browser:identity": "work",
        "browser:sleep": { since: 3, url: "https://a.example/", title: "A", reason: "pressure", kept: true },
        "browser:loadms": 1234,
        "browser:keepawake": true,
        "browser:allowed_origins": ["https://a.example", 4, "https://b.example"],
        "browser:popup_from": " https://opener.example ",
        "browser:popup_windows": [{ id: "p1", url: "https://p.example" }, { id: 2, url: "x" }, { id: "p3" }],
    };

    it("reads every key, and tolerates a wrong shape in each", async () => {
        const m = build(full);
        const frames = [`## full\n${atoms(m.vm)}`];
        m.setMetaSignal({
            "browser:show_controls": "yes",
            "browser:attention": { kind: "no id" },
            "browser:actions": { entries: "no" },
            "browser:paused": "true",
            "browser:owner_agent": "   ",
            "browser:given_back": true,
            "browser:last_driver": "Ada",
            "browser:identity": "",
            "browser:sleep": "asleep",
            "browser:loadms": -1,
            "browser:keepawake": 1,
            "browser:allowed_origins": "all",
            "browser:popup_from": "  ",
            "browser:popup_windows": {},
        });
        frames.push(`## wrong shapes\n${atoms(m.vm)}`);
        m.setMetaSignal({});
        frames.push(`## empty\n${atoms(m.vm)}`);
        m.setMetaSignal({ "browser:given_back": false, "browser:last_driver": "Ada", "browser:bypass": { ended: "time ran out" } });
        frames.push(`## last driver kept, bypass ended\n${atoms(m.vm)}`);
        await expect(frames.join("\n") + "\n" + out()).toMatchFileSnapshot(snap("atoms-from-meta"));
    });

    it("shows a bypass while it lasts, ticks its clock, and stops the clock when it ends", async () => {
        const now = Date.now();
        const m = build({ "browser:bypass": { expires_at: now + 40_000, origins: ["https://a.example"] } });
        const frames: string[] = [];
        const frame = (l: string) => frames.push(`## ${l} timers=${vi.getTimerCount()}\nbypass=${sv(m.vm.bypassAtom())}`);
        frame("granted");
        await settle(15_000);
        frame("15 s");
        await settle(30_000);
        frame("45 s: expired");
        m.setMetaSignal({ "browser:bypass": { expires_at: now + 1000, origins: [], paused_left: 5000 } });
        await settle(60_000);
        frame("paused: stands still");
        m.setMetaSignal({ "browser:bypass": { expires_at: "soon", origins: [] } });
        frame("malformed");
        m.setMetaSignal({ "browser:bypass": { expires_at: now + 10, origins: "all" } });
        frame("origins not a list");
        m.setMetaSignal({});
        frame("revoked");
        m.vm.dispose();
        frame("disposed");
        await expect(frames.join("\n") + "\n") .toMatchFileSnapshot(snap("bypass-clock"));
    });

    it("traces the favicon the view reads, and the title's placeholder state", async () => {
        const m = build({ url: "https://a.example/" });
        await settle();
        fire("browser-pane-favicon-urls", { block_id: BLOCK, urls: ["https://a.example/f.ico", "https://a.example/g.png"] });
        fire("browser-pane-title-change", { block_id: BLOCK, title: "Real title" });
        log.push(`atoms ${atoms(m.vm)}`);
        await expect(out()).toMatchFileSnapshot(snap("view-name-and-favicon"));
    });
});

describe("the host's events", () => {
    it("title and favicon events: only the pane they are for answers, and none after dispose", async () => {
        const m = build({ url: "https://a.example/" });
        await settle();
        log.push("-- other pane's events");
        fire("browser-pane-title-change", { block_id: "someone-else", title: "X" });
        fire("browser-pane-favicon-urls", { block_id: "someone-else", urls: ["https://x/f.ico"] });
        log.push("-- this pane's");
        fire("browser-pane-title-change", { block_id: BLOCK, title: "Hello" });
        fire("browser-pane-favicon-urls", { block_id: BLOCK, urls: [] });
        log.push(`atoms ${atoms(m.vm)}`);
        log.push("-- after dispose");
        m.vm.dispose();
        fire("browser-pane-title-change", { block_id: BLOCK, title: "Late" });
        fire("browser-pane-favicon-urls", { block_id: BLOCK, urls: ["https://late/f.ico"] });
        await expect(out()).toMatchFileSnapshot(snap("title-favicon-events"));
    });

    it("nav-state: confirms the url, takes back/forward unless url_only, and holds the end of a load briefly", async () => {
        const m = build({ url: "https://a.example/" });
        await settle();
        const nav = (p: Record<string, unknown>) => {
            fire("browser-pane-nav-state", { block_id: BLOCK, url: "https://a.example/next", ...p });
        };
        log.push("-- another pane's");
        fire("browser-pane-nav-state", { block_id: "someone-else", url: "https://x/" });
        log.push("-- url only");
        nav({ url_only: true, can_go_back: true });
        log.push(`atoms ${atoms(m.vm)}`);
        log.push("-- back and forward");
        nav({ can_go_back: true, can_go_forward: false });
        nav({ can_go_forward: true });
        log.push(`atoms ${atoms(m.vm)}`);
        log.push("-- loading starts: at once");
        nav({ is_loading: true, can_go_back: true, can_go_forward: true });
        log.push(`loading=${m.vm.loadingAtom()}`);
        log.push("-- loading ends: held 200 ms");
        nav({ is_loading: false });
        log.push(`loading=${m.vm.loadingAtom()}`);
        await settle(100);
        log.push(`after 100 ms loading=${m.vm.loadingAtom()}`);
        log.push("-- a fresh start cancels the held end");
        nav({ is_loading: true });
        await settle(500);
        log.push(`after 500 ms loading=${m.vm.loadingAtom()}`);
        nav({ is_loading: false, can_go_back: false });
        await settle(250);
        log.push(`after the hold loading=${m.vm.loadingAtom()}`);
        log.push("-- no tab to load: nothing");
        log.push("-- after dispose");
        m.vm.dispose();
        nav({ is_loading: true });
        log.push(`slot ${slot()}`);
        await expect(out()).toMatchFileSnapshot(snap("nav-state-events"));
    });

    it("a failed load shows its reason and stops the loading state", async () => {
        const m = build({ url: "https://a.example/" });
        await settle();
        m.vm.onError("net::ERR_CONNECTION_REFUSED");
        log.push(`atoms ${atoms(m.vm)}`);
        await expect(out()).toMatchFileSnapshot(snap("load-failed"));
    });

    it("a click blurs a text box the page's click never reached, then focuses the pane", async () => {
        const m = build({ url: "https://a.example/" });
        await settle();
        const input = document.createElement("input");
        input.className = "address";
        document.body.appendChild(input);
        input.focus();
        fire("browser-pane-clicked", { block_id: "someone-else" });
        log.push("-- clicked");
        fire("browser-pane-clicked", { block_id: BLOCK });
        log.push(`input still focused: ${document.activeElement === input}`);
        const dummy = document.createElement("input");
        dummy.className = "dummy-focus";
        document.body.appendChild(dummy);
        dummy.focus();
        fire("browser-pane-clicked", { block_id: BLOCK });
        log.push(`dummy still focused: ${document.activeElement === dummy}`);
        input.remove();
        dummy.remove();
        m.vm.dispose();
        fire("browser-pane-clicked", { block_id: BLOCK });
        await expect(out()).toMatchFileSnapshot(snap("click-events"));
    });

    it("a page that closes itself closes its pane or tab, once and only for this pane", async () => {
        const m = build({ url: "https://a.example/" });
        await settle();
        fire("browser-pane-close-requested", { block_id: "someone-else" });
        fire("browser-pane-close-requested", { block_id: BLOCK });
        await settle();
        m.vm.dispose();
        fire("browser-pane-close-requested", { block_id: BLOCK });
        await settle();
        await expect(out()).toMatchFileSnapshot(snap("close-requested"));
    });

    it("a notice from the host is shown when srv has none up, and srv's own banner wins", async () => {
        const m = build({ url: "https://a.example/" });
        await settle();
        fire("browser-pane-notice", { block_id: "someone-else", url: "https://x/" });
        fire("browser-pane-notice", { block_id: BLOCK, url: "https://blocked.example/path?q=1", sites: ["https://a.example"], why: "Not on the list." });
        log.push(`attention ${sv(m.vm.attentionAtom())}`);
        fire("browser-pane-notice", { block_id: BLOCK, url: "not a url" });
        log.push(`attention ${sv(m.vm.attentionAtom())}`);
        m.setMetaSignal({ "browser:attention": { id: "srv-1", kind: "handoff" } });
        log.push(`attention ${sv(m.vm.attentionAtom())}`);
        m.vm.dispose();
        fire("browser-pane-notice", { block_id: BLOCK, url: "https://late/" });
        log.push(`attention ${sv(m.vm.attentionAtom())}`);
        await expect(out()).toMatchFileSnapshot(snap("notice-events"));
    });

    it("download updates fill the strip, and a finished row folds away after its time", async () => {
        const m = build({ url: "https://a.example/" });
        await settle();
        const dl = (p: Record<string, unknown>) => fire("pane-download-update", { block_id: BLOCK, ...p });
        fire("pane-download-update", { block_id: "someone-else", download_id: 9, state: "done" });
        dl({ download_id: 1, state: "started", name: "a.zip", path: "/tmp/a.zip", received: 0, total: 100 });
        dl({ download_id: 1, state: "progress", name: "a.zip", path: "/tmp/a.zip", received: 50, total: 100 });
        dl({ download_id: 2, state: "started", name: "b.pdf", path: "/tmp/b.pdf" });
        log.push(`rows ${sv(m.vm.downloadsAtom())}`);
        dl({ download_id: 1, state: "done", name: "a.zip", path: "/tmp/a.zip" });
        log.push(`rows ${sv(m.vm.downloadsAtom().map((r: any) => r.state))} timers=${vi.getTimerCount()}`);
        dl({ download_id: 1, state: "done", name: "a.zip", path: "/tmp/a.zip" });
        log.push(`timers=${vi.getTimerCount()} after a repeat`);
        await settle(60_000);
        log.push(`rows ${sv(m.vm.downloadsAtom().map((r: any) => r.state))}`);
        dl({ download_id: 2, state: "done", name: "b.pdf", path: "/tmp/b.pdf" });
        log.push(`timers=${vi.getTimerCount()}`);
        m.vm.dispose();
        log.push(`timers=${vi.getTimerCount()} after dispose`);
        dl({ download_id: 3, state: "started" });
        await expect(out()).toMatchFileSnapshot(snap("download-events"));
    });
});
