// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a browser pane does around its page, pinned as ordered logs: how it goes to sleep and wakes
 * (launch, hover, press, an agent's call, being shown), how it freezes and thaws, how it times a load,
 * and the loading overlay's mount, fade and badge. The real BrowserViewComponent and the real
 * hibernation module run over a stand-in model and host; the native-pane hooks are stubs that report
 * what they were given. Each test logs every model call, host call, diagnostic line and probe in order.
 * A refactor of the view must leave the committed snapshots as they are; regenerate with `vitest -u`
 * only for a change meant to behave differently.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { makeTestHostApi } from "@/app/host/test-host";

const state = vi.hoisted(() => ({
    log: [] as string[],
    settings: {} as Record<string, unknown>,
    launchMount: false,
    reduceMotion: false,
    registered: [] as any[],
    rect: undefined as any,
    drag: undefined as any,
}));

vi.mock("@/element/modal-layer", () => ({ useModalLayer: () => ({}) }));
vi.mock("@/app/block/pane-tab-visibility", () => ({ usePaneTabVisibility: () => (globalThis as any).__visibility }));
vi.mock("@/app/workspace/window-tab-visibility", () => ({ useWindowTabId: () => "t1" }));
vi.mock("@/store/global", () => ({ getSettingsKeyAtom: (k: string) => () => state.settings[k] }));
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: (v: string) => state.log.push(`open pane ${v}`) }));
vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: () => {} }));
vi.mock("@/app/motion/motion", () => ({ scaledMs: (ms: number) => ms, shouldReduceMotion: () => state.reduceMotion }));
vi.mock("./use-pane-rect-sync", () => ({
    usePaneRectSync: (p: unknown) => {
        state.rect = p;
        return (globalThis as any).__rectSync;
    },
}));
vi.mock("./use-freeze-frame", () => ({
    useFreezeFrame: () => ({ freezeSnapshot: () => (globalThis as any).__freezeSnapshot(), freezeStyle: () => ({ top: "1px" }) }),
}));
vi.mock("./use-drag-snapshot", () => ({
    useDragSnapshot: (p: unknown) => {
        state.drag = p;
        return { take: async () => (globalThis as any).__dragSnapshot() };
    },
}));
vi.mock("./use-browser-auth", () => ({ useBrowserAuth: () => {} }));
vi.mock("./browser-nav-bar", () => ({
    BrowserNavBar: (p: { windowLabel: string }) => <div class="stub-nav" data-window-label={p.windowLabel} />,
}));
vi.mock("./browser-signin-offer", () => ({
    BrowserSigninOffer: (p: { covered: () => boolean }) => <div class="stub-offer" data-covered={String(p.covered())} />,
    BrowserSigninPending: () => <div class="stub-pending" />,
}));
vi.mock("./download-strip", () => ({ DownloadStrip: () => <div class="stub-downloads" /> }));
vi.mock("./driven-by-strip", () => ({ DrivenByStrip: (p: { agent: string }) => <div class="stub-driven" data-agent={p.agent} /> }));
vi.mock("./give-back-strip", () => ({ NoDriverStrips: () => <div class="stub-no-driver" /> }));
vi.mock("./browser-attention", () => ({ BrowserAttentionBanner: (p: { attention: () => { kind: string } }) => <div class="stub-attention" data-kind={p.attention().kind} /> }));
vi.mock("./browser-popups", () => ({ BrowserPopupNotes: () => <div class="stub-popups" /> }));
vi.mock("@/app/drag/drag-session", () => ({ isAnyUnderway: () => (globalThis as any).__underway() }));
vi.mock("@/app/element/BrainSpinner", () => ({ BrainSpinner: (p: { class?: string }) => <i class={`stub-brain ${p.class ?? ""}`} /> }));
vi.mock("./hibernation", async (importOriginal) => {
    const real = await importOriginal<typeof import("./hibernation")>();
    return {
        ...real,
        isLaunchMount: () => state.launchMount,
        registerHibernatablePane: (p: unknown) => {
            state.registered.push(p);
            state.log.push("register pane");
            return () => state.log.push("unregister pane");
        },
    };
});

import { BrowserViewComponent } from "./browser-view";
import { leaveTabPill, pressTabPill, resetEngineSupportsFreeze, WAKE_META_KEY, type BrowserSleep } from "./hibernation";

const URL = "https://example.com/page";

interface Opts {
    visible?: boolean;
    sleep?: Partial<BrowserSleep> | null;
    url?: string;
    paneCreated?: boolean;
    hibernateFails?: boolean;
    check?: unknown;
    setFrozen?: (frozen: boolean) => Promise<boolean>;
    discard?: unknown;
    memory?: Record<string, number> | null;
    dragSnapshot?: { src: string; style: Record<string, string> } | null;
}

const sleepRecord = (over: Partial<BrowserSleep> = {}): BrowserSleep => ({
    since: 1,
    reason: "hidden",
    url: URL,
    title: "Example",
    favicon: "",
    ...over,
});

/** Mounts the view over a stand-in model; every call the view makes is appended to state.log. */
function mount(o: Opts = {}) {
    const log = (s: string) => state.log.push(s);
    const [visible, setVisible] = createSignal(o.visible ?? true);
    (globalThis as any).__visibility = () => (visible() ? "active" : "hidden");
    const [paneCreated, setPaneCreated] = createSignal(o.paneCreated ?? false);
    (globalThis as any).__rectSync = { paneCreated, createPane: async () => {}, paneRect: () => null };
    const [underway, setUnderway] = createSignal(false);
    (globalThis as any).__underway = underway;
    (globalThis as any).__dragSnapshot = () => o.dragSnapshot ?? null;
    const [freezeSnap, setFreezeSnap] = createSignal<string | null>(null);
    (globalThis as any).__freezeSnapshot = freezeSnap;
    const [url, setUrl] = createSignal(o.url ?? URL);
    const [title, setTitle] = createSignal("Example");
    const [favicon, setFavicon] = createSignal("");
    const [loading, setLoading] = createSignal(false);
    const [sleep, setSleep] = createSignal<BrowserSleep | undefined>(o.sleep ? sleepRecord(o.sleep) : undefined);
    const [keepAwake, setKeepAwake] = createSignal(false);
    const [driver, setDriver] = createSignal<string | undefined>(undefined);
    const [attention, setAttention] = createSignal<{ kind: string } | undefined>(undefined);
    const [popups, setPopups] = createSignal<unknown[]>([]);
    const [error, setError] = createSignal<string | undefined>(undefined);
    const [meta, setMeta] = createSignal<Record<string, unknown>>({});
    const [entries, setEntries] = createSignal<{ ts: number }[]>([]);
    const model: any = {
        blockId: "blk-1234567",
        closed: false,
        urlAtom: url,
        titleAtom: title,
        faviconUrlAtom: favicon,
        loadingAtom: loading,
        sleepAtom: sleep,
        keepAwakeAtom: keepAwake,
        driverAgentAtom: driver,
        bypassAtom: () => undefined,
        attentionAtom: attention,
        popupWindowsAtom: popups,
        errorAtom: error,
        actionsAtom: () => ({ count: entries().length, entries: entries() }),
        meta,
        hibernate: async (reason: string, memory?: number) => {
            log(`model.hibernate ${reason} memory=${memory}`);
            if (o.hibernateFails) throw new Error("no");
            setSleep(sleepRecord({ reason: reason as BrowserSleep["reason"], memoryBytes: memory, kept: true }));
        },
        endSleep: async (ms?: number) => {
            log(`model.endSleep ${ms === undefined ? "undefined" : Math.round(ms)}`);
            setSleep(undefined);
        },
        clearSleepKept: async () => log("model.clearSleepKept"),
        recordLoadMs: async (ms: number) => log(`model.recordLoadMs ${Math.round(ms)}`),
        keepAwake: async () => log("model.keepAwake"),
    };
    window.api = makeTestHostApi(
        {
            browserPanes: {
                setFrozen: async (id: string, f: boolean) => {
                    log(`host.setFrozen ${f}`);
                    return o.setFrozen ? o.setFrozen(f) : true;
                },
                hibernationCheck: async (_id: string, mode: string) => {
                    log(`host.hibernationCheck ${mode}`);
                    return o.check === undefined ? { live: true } : o.check;
                },
                paneMemory: async () => {
                    log("host.paneMemory");
                    return o.memory === null ? undefined : (o.memory ?? { "blk-1234567": 4096 });
                },
                discard: async (_id: string, what: string) => {
                    log(`host.discard ${what}`);
                    return o.discard === undefined ? { supported: true } : o.discard;
                },
                focus: async (id: string) => log(`host.focus ${id}`),
            } as unknown as BrowserPaneHostApi,
        },
        { nativeBrowserPane: true },
    );
    const r = render(() => <BrowserViewComponent model={model} />);
    const pane = () => state.registered[state.registered.length - 1];
    const probe = (label: string) => {
        const p = pane();
        log(
            `probe ${label}: rect.asleep=${state.rect.asleep()} drag.unavailable=${state.drag.unavailable()}` +
                (p ? ` pane.visible=${p.isVisible()} pane.asleep=${p.isAsleep()} pane.frozen=${p.isFrozen()} hiddenSince=${p.hiddenSince()}` : ""),
        );
    };
    const settle = (ms = 0) => vi.advanceTimersByTimeAsync(ms);
    return {
        ...r, model, probe, settle, pane,
        setVisible, setPaneCreated, setUnderway, setFreezeSnap, setUrl, setTitle, setFavicon, setLoading, setSleep, setKeepAwake, setDriver, setPopups, setAttention,
        setError, setMeta, setEntries,
        step: (s: string) => log(`-- ${s}`),
        view: () => r.container.querySelector(".browser-view"),
    };
}

/** One element per line, so a snapshot diff points at the element that changed. */
const markup = (el: Element | null): string => (el ? el.outerHTML : "(absent)").replace(/></g, ">\n<");
const snap = (name: string) => `./__snapshots__/browser-view.hibernation.${name}.txt`;
const out = () => state.log.join("\n") + "\n";
/** Unmounts the current pane so the next mount in the same test starts clean; the log carries on. */
const reset = () => {
    state.registered.length = 0;
    cleanup();
};

beforeEach(() => {
    state.log.length = 0;
    state.registered.length = 0;
    state.settings = {};
    state.launchMount = false;
    state.reduceMotion = false;
    resetEngineSupportsFreeze();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date", "performance"] });
    // performance.now() starts at 0 under fake timers, which the view reads as "no wake yet".
    vi.advanceTimersByTime(5000);
    vi.setSystemTime(new Date("2026-11-03T12:00:00Z"));
    vi.spyOn(console, "log").mockImplementation((...a: unknown[]) => {
        const s = String(a[0]);
        if (s.startsWith("[browser-pane:diag]")) state.log.push(`diag ${s.replace(/^\[browser-pane:diag\]\[[^\]]*\] /, "")}`);
    });
});
afterEach(() => {
    cleanup();
    leaveTabPill("t1");
    vi.restoreAllMocks();
    vi.useRealTimers();
});

describe("waking and sleeping", () => {
    it("a pane that mounts hidden as the window loads starts asleep, then wakes when shown and times its load", async () => {
        state.launchMount = true;
        const v = mount({ visible: false, paneCreated: false });
        v.probe("mounted");
        await v.settle();
        v.probe("after hibernate");
        v.step("shown");
        v.setVisible(true);
        await v.settle();
        v.probe("shown");
        v.step("page created and loads");
        v.setPaneCreated(true);
        v.setLoading(true);
        await v.settle(1500);
        v.setLoading(false);
        await v.settle();
        v.probe("loaded");
        v.unmount();
        await expect(out()).toMatchFileSnapshot(snap("launch-asleep"));
    });

    it("a launch hibernate that fails leaves the pane awake", async () => {
        state.launchMount = true;
        const v = mount({ visible: false, hibernateFails: true });
        v.probe("mounted");
        await v.settle();
        v.probe("after failure");
        await expect(out()).toMatchFileSnapshot(snap("launch-asleep-fails"));
    });

    it("no launch sleep when shown, when the setting is off, when a site is kept awake or the page is blank", async () => {
        state.launchMount = true;
        mount({ visible: true });
        await vi.advanceTimersByTimeAsync(0);
        reset();
        state.launchMount = true;
        state.settings["browser:hibernate"] = false;
        mount({ visible: false });
        state.log.push("-- setting off");
        reset();
        state.launchMount = true;
        state.settings["browser:keepawakesites"] = ["example.com"];
        mount({ visible: false });
        state.log.push("-- site kept awake");
        reset();
        state.launchMount = true;
        mount({ visible: false, url: "" });
        state.log.push("-- blank");
        reset();
        state.launchMount = true;
        mount({ visible: false, sleep: { reason: "hidden" } });
        state.log.push("-- already asleep");
        const v = mount({ visible: false });
        v.probe("last");
        await v.settle();
        v.probe("last settled");
        await expect(out()).toMatchFileSnapshot(snap("launch-not-taken"));
    });

    it("a kept sleep record on a pane that mounts asleep is cleared, an unkept one is not", async () => {
        mount({ visible: true, sleep: { kept: true } });
        mount({ visible: true, sleep: { kept: false } });
        await expect(out()).toMatchFileSnapshot(snap("sleep-kept-cleared"));
    });

    it("a hover on the tab wakes a hidden pane, which goes back to sleep after ten seconds with no switch", async () => {
        const v = mount({ visible: false, sleep: {}, paneCreated: true });
        v.probe("asleep");
        pressTabPill("t1");
        await v.settle();
        v.probe("intent wake");
        v.step("load finishes while hidden");
        v.setLoading(true);
        await v.settle(300);
        v.setLoading(false);
        await v.settle();
        v.probe("loaded hidden");
        v.step("ten seconds, no switch");
        await v.settle(10_000);
        v.probe("abandoned");
        await expect(out()).toMatchFileSnapshot(snap("intent-abandoned"));
    });

    it("a hover wake that has loaded is finished when the pane is shown, with no card", async () => {
        const v = mount({ visible: false, sleep: {}, paneCreated: true });
        pressTabPill("t1");
        await v.settle();
        v.setLoading(true);
        await v.settle(420);
        v.setLoading(false);
        await v.settle();
        v.step("shown");
        v.setVisible(true);
        await v.settle();
        v.probe("shown");
        await expect(out()).toMatchFileSnapshot(snap("intent-then-shown"));
    });

    it("an agent's call upgrades a hover wake to a real one, and a second hover or call changes nothing", async () => {
        const v = mount({ visible: false, sleep: {}, paneCreated: true });
        pressTabPill("t1");
        await v.settle();
        v.step("agent call asks for a wake");
        v.setMeta({ [WAKE_META_KEY]: 1 });
        await v.settle();
        v.setMeta({ [WAKE_META_KEY]: 2 });
        await v.settle();
        v.step("load finishes while hidden: an agent's wake ends at once");
        v.setLoading(true);
        await v.settle(250);
        v.setLoading(false);
        await v.settle();
        v.probe("end");
        await expect(out()).toMatchFileSnapshot(snap("agent-upgrade"));
    });

    it("an agent's call upgrades a hover wake that has already loaded", async () => {
        const v = mount({ visible: false, sleep: {}, paneCreated: true });
        pressTabPill("t1");
        await v.settle();
        v.setLoading(true);
        await v.settle(100);
        v.setLoading(false);
        await v.settle();
        v.step("agent call after the load");
        v.setMeta({ [WAKE_META_KEY]: 1 });
        await v.settle();
        v.probe("end");
        await expect(out()).toMatchFileSnapshot(snap("agent-upgrade-loaded"));
    });

    it("a wake asked of an awake pane is ignored, and a command upgrades a hover wake", async () => {
        const v = mount({ visible: true, sleep: null });
        v.pane().wake("show");
        v.setVisible(false);
        v.setSleep(sleepRecord());
        await v.settle();
        pressTabPill("t1");
        await v.settle();
        v.step("command");
        v.pane().wake("command");
        v.step("agent after command: nothing");
        v.pane().wake("agent");
        await v.settle();
        v.probe("end");
        await expect(out()).toMatchFileSnapshot(snap("wake-command"));
    });

    it("a hover wake whose pane is shown before the ten seconds are up is left alone", async () => {
        const v = mount({ visible: false, sleep: {}, paneCreated: true });
        pressTabPill("t1");
        await v.settle();
        v.setVisible(true);
        await v.settle(11_000);
        v.probe("end");
        await expect(out()).toMatchFileSnapshot(snap("intent-shown-before-give-up"));
    });

    it("an agent's call upgrades a hover wake that ended without a load", async () => {
        const v = mount({ visible: false, sleep: {}, paneCreated: true });
        pressTabPill("t1");
        await v.settle();
        state.rect.onWakeWithoutLoad();
        await v.settle();
        v.pane().wake("agent");
        await v.settle();
        v.probe("end");
        await expect(out()).toMatchFileSnapshot(snap("agent-upgrade-no-load"));
    });

    it("a wake that needs no load ends at once when shown, and waits ready when hidden", async () => {
        const v = mount({ visible: true, sleep: {} });
        await v.settle();
        v.step("shown pane: onWakeWithoutLoad");
        state.rect.onWakeWithoutLoad();
        await v.settle();
        reset();

        const h = mount({ visible: false, sleep: {}, paneCreated: true });
        pressTabPill("t1");
        await h.settle();
        h.step("hidden pane: onWakeWithoutLoad");
        state.rect.onWakeWithoutLoad();
        await h.settle();
        h.probe("ready");
        h.step("shown");
        h.setVisible(true);
        await h.settle();
        h.probe("shown");
        h.step("not asleep: onWakeWithoutLoad is ignored");
        state.rect.onWakeWithoutLoad();
        await expect(out()).toMatchFileSnapshot(snap("wake-without-load"));
    });

    it("a page created while asleep times its wake from the wake's start, and a wake that ends unloaded resets the clock", async () => {
        const v = mount({ visible: false, sleep: {}, paneCreated: false });
        await v.settle(500);
        pressTabPill("t1");
        await v.settle(200);
        v.step("page created at wake");
        v.setPaneCreated(true);
        v.setLoading(true);
        await v.settle(700);
        v.setLoading(false);
        await v.settle();
        v.probe("loaded");
        v.step("page lost");
        v.setPaneCreated(false);
        await v.settle();
        v.step("a plain load while awake is recorded");
        v.setSleep(undefined);
        v.setPaneCreated(true);
        v.setLoading(true);
        await v.settle(333);
        v.setLoading(false);
        await v.settle();
        v.step("a load with no loading seen");
        v.setPaneCreated(false);
        v.setPaneCreated(true);
        v.setLoading(false);
        await v.settle();
        await expect(out()).toMatchFileSnapshot(snap("load-clock"));
    });

    it("the card is shown while a shown pane wakes and says how long it was up", async () => {
        const v = mount({ visible: false, sleep: {}, paneCreated: true });
        await v.settle();
        v.step("shown");
        v.setVisible(true);
        await v.settle(900);
        v.step("hidden again mid-wake");
        v.setVisible(false);
        await v.settle();
        v.step("shown again");
        v.setVisible(true);
        await v.settle();
        await expect(out()).toMatchFileSnapshot(snap("card-diag"));
    });
});

describe("hibernating", () => {
    const run = async (v: ReturnType<typeof mount>, reason: any = "hidden") => {
        const r = await v.pane().tryHibernate(reason);
        state.log.push(`tryHibernate -> ${JSON.stringify(r)}`);
        await v.settle();
    };

    it("what the window knows keeps a pane awake, and nothing else is asked of the page", async () => {
        const v = mount({ visible: false, sleep: null });
        await v.settle();
        v.setKeepAwake(true);
        await run(v);
        v.setKeepAwake(false);
        v.setUrl("");
        await run(v);
        v.setUrl("http://localhost:3000/");
        await run(v);
        state.settings["browser:keepawakesites"] = ["example.com"];
        v.setUrl(URL);
        await run(v);
        state.settings["browser:keepawakesites"] = undefined;
        v.setPopups([{}]);
        await run(v);
        v.setPopups([]);
        v.setDriver("Korp");
        v.setEntries([{ ts: Date.now() - 1000 }]);
        await run(v);
        v.setDriver(undefined);
        v.setMeta({ "browser:lastcall": Date.now() - 2000 });
        await run(v);
        await expect(out()).toMatchFileSnapshot(snap("hibernate-declined-by-window"));
    });

    it("a visible or sleeping pane is left alone, and a page that answers 'in use' keeps it awake", async () => {
        const v = mount({ visible: true, sleep: null });
        await run(v);
        v.setVisible(false);
        v.setSleep(sleepRecord());
        await run(v);
        v.setSleep(undefined);
        await v.settle();
        for (const check of [{ live: true, edited: true }, { live: true, beforeunload: true }, { live: true, media: true }, { live: false }, null]) {
            v.step(`check ${JSON.stringify(check)}`);
            (window.api as any).browserPanes.hibernationCheck = async (_i: string, mode: string) => {
                state.log.push(`host.hibernationCheck ${mode}`);
                return check;
            };
            await run(v);
        }
        await expect(out()).toMatchFileSnapshot(snap("hibernate-declined-by-page"));
    });

    it("a hidden idle pane goes to sleep with its memory, and a pane shown meanwhile does not", async () => {
        const v = mount({ visible: false, sleep: null });
        await run(v, "pressure");
        v.probe("asleep");
        v.step("shown during the check");
        v.setSleep(undefined);
        (window.api as any).browserPanes.hibernationCheck = async () => {
            state.log.push("host.hibernationCheck during");
            v.setVisible(true);
            return { live: true };
        };
        await run(v);
        v.step("shown during the memory read");
        v.setVisible(false);
        await v.settle();
        (window.api as any).browserPanes.hibernationCheck = async () => ({ live: true });
        (window.api as any).browserPanes.paneMemory = async () => {
            state.log.push("host.paneMemory during");
            v.setVisible(true);
            return {};
        };
        await run(v);
        v.step("fell asleep during the check");
        v.setVisible(false);
        await v.settle();
        (window.api as any).browserPanes.hibernationCheck = async () => {
            v.setSleep(sleepRecord());
            return { live: true };
        };
        await run(v);
        await expect(out()).toMatchFileSnapshot(snap("hibernate-races"));
    });

    it("a repeated refusal is said once, and a frozen page that is kept awake is put back frozen", async () => {
        const v = mount({ visible: false, sleep: null });
        v.pane();
        (window.api as any).browserPanes.hibernationCheck = async (_i: string, mode: string) => {
            state.log.push(`host.hibernationCheck ${mode}`);
            return { live: true, edited: true };
        };
        await run(v);
        await run(v);
        v.step("freeze it first");
        (window.api as any).browserPanes.hibernationCheck = async () => ({ live: true });
        const f = await v.pane().tryFreeze();
        state.log.push(`tryFreeze -> ${JSON.stringify(f)}`);
        v.probe("frozen");
        v.step("hibernate a frozen page that has since typed text");
        (window.api as any).browserPanes.hibernationCheck = async (_i: string, mode: string) => {
            state.log.push(`host.hibernationCheck ${mode}`);
            return { live: true, edited: true };
        };
        await run(v);
        v.probe("frozen again");
        v.step("asked again: the remembered reason answers without a thaw");
        await run(v);
        v.step("shown and hidden again: the thaw clears the remembered reason, and the page sleeps");
        v.setVisible(true);
        await v.settle();
        v.setVisible(false);
        await v.settle();
        (window.api as any).browserPanes.hibernationCheck = async (_i: string, mode: string) => {
            state.log.push(`host.hibernationCheck ${mode}`);
            return { live: true };
        };
        await run(v);
        v.probe("end");
        await expect(out()).toMatchFileSnapshot(snap("hibernate-frozen"));
    });

    it("a frozen page that is kept awake and shown while being put back is thawed", async () => {
        const v = mount({ visible: false, sleep: null });
        await v.pane().tryFreeze();
        (window.api as any).browserPanes.hibernationCheck = async () => ({ live: true, edited: true });
        (window.api as any).browserPanes.setFrozen = async (_i: string, f: boolean) => {
            state.log.push(`host.setFrozen ${f}`);
            if (f) v.setVisible(true);
            return true;
        };
        await run(v);
        v.probe("end");
        await expect(out()).toMatchFileSnapshot(snap("hibernate-frozen-shown"));
    });

    it("a title or icon the page changes by itself keeps it awake for a while; the load settling does not", async () => {
        const v = mount({ visible: false, sleep: null });
        v.setLoading(true);
        await v.settle(100);
        v.setTitle("Loading title");
        v.setLoading(false);
        await v.settle(100);
        v.step("title changes right after the load: settling");
        v.setTitle("Settled");
        await run(v);
        v.step("title changes later: the page is doing something");
        await v.settle(6000);
        v.setTitle("Inbox (1)");
        await run(v);
        v.step("icon changes");
        await v.settle(31 * 60_000);
        v.setFavicon("https://example.com/a.ico");
        await v.settle();
        v.setFavicon("https://example.com/b.ico");
        await run(v);
        v.step("shown: the baseline is reset");
        v.setVisible(true);
        await v.settle();
        v.setVisible(false);
        await v.settle();
        await run(v);
        await expect(out()).toMatchFileSnapshot(snap("hibernate-activity"));
    });

    it("hiddenSince follows the pane being hidden and shown", async () => {
        const v = mount({ visible: true, sleep: null });
        v.probe("visible");
        await v.settle(5000);
        v.setVisible(false);
        await v.settle();
        v.probe("hidden");
        await v.settle(7000);
        v.setVisible(true);
        await v.settle();
        v.probe("shown");
        const hidden = mount({ visible: false });
        hidden.probe("mounted hidden");
        await expect(out()).toMatchFileSnapshot(snap("hidden-since"));
    });
});

describe("freezing", () => {
    const freeze = async (v: ReturnType<typeof mount>) => {
        const r = await v.pane().tryFreeze();
        state.log.push(`tryFreeze -> ${JSON.stringify(r)}`);
        await v.settle();
    };

    it("freezes a hidden idle pane and refuses each reason not to", async () => {
        const v = mount({ visible: true, sleep: null });
        await freeze(v);
        v.setVisible(false);
        v.setSleep(sleepRecord());
        await freeze(v);
        v.setSleep(undefined);
        v.setKeepAwake(true);
        await freeze(v);
        v.setKeepAwake(false);
        (window.api as any).browserPanes.hibernationCheck = async (_i: string, mode: string) => {
            state.log.push(`host.hibernationCheck ${mode}`);
            return { live: true, media: true };
        };
        await freeze(v);
        (window.api as any).browserPanes.hibernationCheck = async () => null;
        await freeze(v);
        (window.api as any).browserPanes.setFrozen = async (_i: string, f: boolean) => {
            state.log.push(`host.setFrozen ${f}`);
            return false;
        };
        (window.api as any).browserPanes.hibernationCheck = async () => ({ live: true });
        await freeze(v);
        (window.api as any).browserPanes.setFrozen = async (_i: string, f: boolean) => {
            state.log.push(`host.setFrozen ${f}`);
            return true;
        };
        await freeze(v);
        await freeze(v);
        v.probe("frozen");
        v.step("shown: thawed");
        v.setVisible(true);
        await v.settle();
        v.probe("shown");
        await expect(out()).toMatchFileSnapshot(snap("freeze-matrix"));
    });

    it("a host without the freeze, discard, check and memory calls freezes nothing and sleeps without a figure", async () => {
        const v = mount({ visible: false, sleep: null });
        const api = (window.api as any).browserPanes;
        for (const k of ["setFrozen", "discard", "hibernationCheck", "paneMemory"]) delete api[k];
        await freeze(v);
        const r = await v.pane().tryHibernate("hidden");
        state.log.push(`tryHibernate -> ${JSON.stringify(r)}`);
        await expect(out()).toMatchFileSnapshot(snap("bare-host"));
    });

    it("a host that stops answering thaws nothing, and a missing memory figure is passed as none", async () => {
        const v = mount({ visible: false, memory: null });
        await freeze(v);
        delete (window.api as any).browserPanes.setFrozen;
        v.setVisible(true);
        await v.settle();
        v.probe("shown");
        reset();
        const w = mount({ visible: false, sleep: null, memory: null });
        const r = await w.pane().tryHibernate("manual");
        state.log.push(`tryHibernate -> ${JSON.stringify(r)}`);
        await expect(out()).toMatchFileSnapshot(snap("host-gaps"));
    });

    it("a pane shown while the freeze check is out is not frozen", async () => {
        const v = mount({ visible: false });
        (window.api as any).browserPanes.hibernationCheck = async () => {
            state.log.push("host.hibernationCheck during");
            v.setVisible(true);
            return { live: true };
        };
        await freeze(v);
        v.step("asleep while the check was out");
        v.setVisible(false);
        await v.settle();
        (window.api as any).browserPanes.hibernationCheck = async () => {
            v.setSleep(sleepRecord());
            return { live: true };
        };
        await freeze(v);
        await expect(out()).toMatchFileSnapshot(snap("freeze-races"));
    });

    it("hibernating a frozen page: shown during the thaw, and a refused re-freeze", async () => {
        const v = mount({ visible: false });
        await freeze(v);
        (window.api as any).browserPanes.setFrozen = async (_i: string, f: boolean) => {
            state.log.push(`host.setFrozen ${f}`);
            if (!f) v.setVisible(true);
            return true;
        };
        const r = await v.pane().tryHibernate("hidden");
        state.log.push(`tryHibernate -> ${JSON.stringify(r)}`);
        reset();
        const w = mount({ visible: false });
        await freeze(w);
        (window.api as any).browserPanes.hibernationCheck = async () => ({ live: true, edited: true });
        (window.api as any).browserPanes.setFrozen = async (_i: string, f: boolean) => {
            state.log.push(`host.setFrozen ${f}`);
            return f ? false : true;
        };
        const r2 = await w.pane().tryHibernate("hidden");
        state.log.push(`tryHibernate -> ${JSON.stringify(r2)}`);
        w.probe("end");
        await expect(out()).toMatchFileSnapshot(snap("hibernate-frozen-gaps"));
    });

    it("an engine that cannot show a frozen page again is not asked to", async () => {
        const v = mount({ visible: false, discard: { supported: false } });
        await freeze(v);
        await expect(out()).toMatchFileSnapshot(snap("freeze-unsupported"));
    });

    it("a pane shown while the freeze call is out is thawed at once", async () => {
        const v = mount({ visible: false });
        (window.api as any).browserPanes.setFrozen = async (_i: string, f: boolean) => {
            state.log.push(`host.setFrozen ${f}`);
            if (f) v.setVisible(true);
            return true;
        };
        await freeze(v);
        v.probe("end");
        await expect(out()).toMatchFileSnapshot(snap("freeze-then-shown"));
    });

    it("a pane shown, or its tab pressed, thaws; a failed thaw is tried again up to five times", async () => {
        let fail = true;
        const v = mount({ visible: false });
        await freeze(v);
        (window.api as any).browserPanes.setFrozen = async (_i: string, f: boolean) => {
            state.log.push(`host.setFrozen ${f}`);
            if (fail) throw new Error("host busy");
            return true;
        };
        v.step("tab pressed");
        pressTabPill("t1");
        await v.settle();
        v.step("shown with the thaw failing");
        v.setVisible(true);
        await v.settle(7000);
        v.probe("gave up");
        v.step("hidden, then shown with the thaw working");
        fail = false;
        v.setVisible(false);
        await v.settle();
        v.setVisible(true);
        await v.settle(2000);
        v.probe("thawed");
        await expect(out()).toMatchFileSnapshot(snap("thaw-retry"));
    });
});

describe("the loading overlay", () => {
    const overlay = (v: ReturnType<typeof mount>) => markup(v.container.querySelector(".browser-placeholder"));

    it("holds the first load's overlay through its fade, then shows a corner badge on later loads", async () => {
        const v = mount({ visible: true });
        const lines: string[] = [];
        const frame = (label: string) => lines.push(`## ${label}\n${overlay(v)}`);
        frame("idle");
        v.setLoading(true);
        await v.settle();
        frame("first load");
        v.setLoading(false);
        await v.settle(10);
        frame("fading");
        await v.settle(2000);
        frame("gone");
        v.setLoading(true);
        await v.settle();
        frame("second load");
        v.setLoading(false);
        await v.settle(10);
        frame("badge fading");
        v.setLoading(true);
        await v.settle();
        frame("loads again mid-fade");
        v.setLoading(false);
        await v.settle(5000);
        frame("done");
        await expect(lines.join("\n") + "\n").toMatchFileSnapshot(snap("overlay-frames"));
    });

    it("unmounts at once when motion is reduced, and a view closed mid-fade clears its timer", async () => {
        state.reduceMotion = true;
        const v = mount({ visible: true });
        const lines: string[] = [];
        v.setLoading(true);
        await v.settle();
        lines.push(overlay(v));
        v.setLoading(false);
        await v.settle();
        lines.push(overlay(v));
        state.reduceMotion = false;
        v.setLoading(true);
        await v.settle();
        v.setLoading(false);
        await v.settle(10);
        lines.push(overlay(v));
        v.unmount();
        await vi.advanceTimersByTimeAsync(5000);
        lines.push(`timers pending: ${vi.getTimerCount()}`);
        await expect(lines.join("\n") + "\n").toMatchFileSnapshot(snap("overlay-reduced"));
    });

    it("no overlay while the sleep card is up", async () => {
        const v = mount({ visible: true, sleep: {}, paneCreated: true });
        v.setLoading(true);
        await v.settle();
        await expect(overlay(v) + "\n").toMatchFileSnapshot(snap("overlay-under-card"));
    });
});

describe("the pane's markup", () => {
    it("draws the empty state, the error, the freeze snapshot and the drag catcher", async () => {
        const v = mount({ visible: true, url: "" });
        const frames: string[] = [];
        frames.push(`## empty\n${markup(v.view())}`);
        v.setError("net::ERR_NAME_NOT_RESOLVED");
        v.setUrl(URL);
        v.setFreezeSnap("data:image/png;base64,AAAA");
        v.setUnderway(true);
        v.setPaneCreated(true);
        await v.settle();
        frames.push(`## error, snapshot\n${markup(v.view())}`);
        await v.settle(600);
        frames.push(`## after the cap\n${markup(v.view())}`);
        await expect(frames.join("\n") + "\n").toMatchFileSnapshot(snap("markup"));
    });

    it("the drag catcher shows the page's snapshot first and opens the hole once it has painted", async () => {
        vi.stubGlobal("requestAnimationFrame", (cb: () => void) => {
            cb();
            return 0;
        });
        const v = mount({ visible: true, dragSnapshot: { src: "data:image/png;base64,BBBB", style: { top: "2px", width: "10px" } } });
        v.setUnderway(true);
        await v.settle(10);
        const lines = [`## snapshot loading\n${markup(v.container.querySelector(".browser-drag-catcher"))}`];
        fireEvent.load(v.container.querySelector(".browser-drag-catcher img")!);
        await v.settle();
        lines.push(`## painted\n${markup(v.container.querySelector(".browser-drag-catcher"))}`);
        v.setUnderway(false);
        await v.settle();
        lines.push(`## drag over\n${markup(v.container.querySelector(".browser-drag-catcher"))}`);
        vi.unstubAllGlobals();
        await expect(lines.join("\n") + "\n").toMatchFileSnapshot(snap("drag-catcher"));
    });

    it("draws the strips and banners around the page in their order, for an agent-driven pane and for one needing the person", async () => {
        const v = mount({ visible: true });
        const frames = [`## nobody drives\n${markup(v.view())}`];
        v.setDriver("Korp");
        v.setAttention({ kind: "handoff" });
        await v.settle();
        frames.push(`## driven, attention\n${markup(v.view())}`);
        await expect(frames.join("\n") + "\n").toMatchFileSnapshot(snap("strips"));
    });

    it("draws the sleep card with its reason, size, ETA and actions, and its buttons act", async () => {
        const v = mount({ visible: true, sleep: { reason: "pressure", memoryBytes: 150 * 1024 * 1024, kept: false, loadMs: 4000, title: "Inbox", favicon: "https://example.com/f.ico" }, paneCreated: true });
        await v.settle(600);
        const card = v.container.querySelector(".browser-sleep-card");
        const lines = [markup(card)];
        const buttons = [...v.container.querySelectorAll(".browser-sleep-card-actions button")];
        for (const b of buttons) fireEvent.click(b);
        lines.push(state.log.join("\n"));
        await v.settle(60_000);
        lines.push(`## much later\n${v.container.querySelector(".browser-sleep-card-eta")?.textContent}`);
        await expect(lines.join("\n") + "\n").toMatchFileSnapshot(snap("sleep-card"));
    });

    it("hands keyboard focus to the page on a press, only while the page exists and the view is open", async () => {
        const v = mount({ visible: true, paneCreated: false });
        const ph = () => v.container.querySelector(".browser-placeholder")!;
        fireEvent.mouseDown(ph());
        v.setPaneCreated(true);
        fireEvent.mouseDown(ph());
        v.model.closed = true;
        fireEvent.mouseDown(ph());
        await v.settle();
        await expect(out()).toMatchFileSnapshot(snap("focus-press"));
    });

    it("tells the hooks what they need, in the order they are built", async () => {
        window.history.pushState({}, "", "/?windowLabel=win-2");
        try {
            const v = mount({ visible: true, sleep: {} });
            await v.settle();
            const lines = [
                `window label ${(v.container.querySelector(".stub-nav") as HTMLElement).dataset.windowLabel}`,
                `rect keys ${Object.keys(state.rect).sort().join(",")}`,
                `rect.model is model ${state.rect.model === v.model}`,
                `drag keys ${Object.keys(state.drag).sort().join(",")}`,
            ];
            await expect(lines.join("\n") + "\n" + out()).toMatchFileSnapshot(snap("hook-args"));
        } finally {
            window.history.pushState({}, "", "/");
        }
    });
});
