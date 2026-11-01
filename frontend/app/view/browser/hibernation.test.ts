// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
const settings: Record<string, unknown> = {};
vi.mock("@/store/global", () => ({ getSettingsKeyAtom: (k: string) => () => settings[k] }));
vi.mock("@/app/store/app-api", () => ({ getApi: () => undefined }));

import {
    ACTIVE_LAPSE_MS,
    afterDiscard,
    createHandlerMemo,
    lastAgentTouch,
    createPageActivity,
    HANDLER_RECHECK_MS,
    LOAD_SETTLE_MS,
    wakeEndsNow,
    awakeReason,
    ENGINE_NO_RECHECK_MS,
    engineSupportsFreeze,
    resetEngineSupportsFreeze,
    etaLabel,
    FREEZE_PER_TICK,
    freezeAwakeReason,
    hibernateAfterMs,
    hibernateHiddenBrowserPanes,
    hoverTabPill,
    isLocalAddress,
    isOverdue,
    leaveTabPill,
    normalizeKeepAwakeSite,
    pageAwakeReason,
    policyTick,
    pressTabPill,
    prewakeTabId,
    readSleep,
    registerHibernatablePane,
    siteKeptAwake,
    sleepName,
    sleepReasonText,
    memoryLabel,
    bySizeThenAge,
    wakeAllBrowserPanes,
    type HibernatablePane,
    type PaneFacts,
    createFreezeState,
} from "./hibernation";

describe("readSleep", () => {
    it("reads a sleep record and drops anything else", () => {
        expect(readSleep(undefined)).toBeUndefined();
        expect(readSleep(null)).toBeUndefined();
        expect(readSleep({ url: "https://x" })).toBeUndefined();
        expect(
            readSleep({ since: 1, url: "https://x", reason: "pressure", title: "X", favicon: "f", loadMs: 900 })
        ).toEqual({
            since: 1,
            url: "https://x",
            reason: "pressure",
            title: "X",
            favicon: "f",
            loadMs: 900,
            kept: false,
        });
        expect(readSleep({ since: 1, url: "https://x", kept: true })?.kept).toBe(true);
    });

    it("treats an unknown reason as manual and a bad load time as none", () => {
        const s = readSleep({ since: 1, url: "https://x", reason: "sneeze", loadMs: -3 });
        expect(s?.reason).toBe("manual");
        expect(s?.loadMs).toBeUndefined();
    });
});

describe("the estimate", () => {
    it("rounds to whole seconds, at least one, and says 'a few' past 3.5 s", () => {
        expect(etaLabel(300)).toBe("About 1 second");
        expect(etaLabel(1400)).toBe("About 1 second");
        expect(etaLabel(2300)).toBe("About 2 seconds");
        expect(etaLabel(3400)).toBe("About 3 seconds");
        expect(etaLabel(3600)).toBe("A few seconds");
    });

    it("uses 2 s for a page with no load time yet", () => {
        expect(etaLabel(undefined)).toBe("About 2 seconds");
    });

    it("is overdue past one and a half times the estimate", () => {
        expect(isOverdue(2000, 2900)).toBe(false);
        expect(isOverdue(2000, 3100)).toBe(true);
        expect(isOverdue(undefined, 3100)).toBe(true);
    });
});

describe("the card's words", () => {
    it("names the page by its title, else its address", () => {
        const base = { since: 1, reason: "manual" as const, favicon: "", url: "https://github.com/a/b/pull/1" };
        expect(sleepName({ ...base, title: "PR 1" })).toBe("PR 1");
        expect(sleepName({ ...base, title: "" })).toBe("github.com/a/b/pull/1");
        expect(sleepName({ ...base, title: "Browser" })).toBe("github.com/a/b/pull/1");
        expect(sleepName({ ...base, url: "https://agentmux.ai/", title: "" })).toBe("agentmux.ai");
    });

    it("says why it slept, and says so when it was memory pressure", () => {
        expect(sleepReasonText("manual")).toMatch(/^AgentMux put this browser pane to sleep to save memory/);
        expect(sleepReasonText("pressure")).toMatch(/^Your computer was low on memory/);
    });

    it("says what the page held when that was measured, else the usual range", () => {
        expect(sleepReasonText("manual")).toContain("usually 100 to 200 MB");
        expect(sleepReasonText("manual", undefined)).toContain("usually 100 to 200 MB");
        expect(sleepReasonText("manual", 0)).toContain("usually 100 to 200 MB");
        expect(sleepReasonText("manual", 183 * 1024 * 1024)).toContain("about 180 MB");
        expect(sleepReasonText("manual", 183 * 1024 * 1024)).not.toContain("usually");
    });

    it("rounds a measured size for a sentence", () => {
        const mb = 1024 * 1024;
        expect(memoryLabel(0.2 * mb)).toBe("1 MB");
        expect(memoryLabel(47.4 * mb)).toBe("47 MB");
        expect(memoryLabel(99.6 * mb)).toBe("100 MB");
        expect(memoryLabel(183 * mb)).toBe("180 MB");
        expect(memoryLabel(999 * mb)).toBe("1000 MB");
        expect(memoryLabel(1024 * mb)).toBe("1.0 GB");
        expect(memoryLabel(1300 * mb)).toBe("1.3 GB");
    });

    it("keeps a measured size in a sleep record and drops a bad one", () => {
        const rec = { since: 1, url: "https://a.test/", reason: "hidden" };
        expect(readSleep({ ...rec, memoryBytes: 1234 })?.memoryBytes).toBe(1234);
        expect(readSleep({ ...rec, memoryBytes: -5 })?.memoryBytes).toBeUndefined();
        expect(readSleep({ ...rec, memoryBytes: "big" })?.memoryBytes).toBeUndefined();
        expect(readSleep(rec)?.memoryBytes).toBeUndefined();
    });
});

describe("wake on a tab pill", () => {
    afterEach(() => {
        vi.useRealTimers();
        leaveTabPill("t1");
    });

    it("hover wakes after a short rest, a press at once, leaving cancels", () => {
        vi.useFakeTimers();
        hoverTabPill("t1");
        expect(prewakeTabId()).toBeNull();
        vi.advanceTimersByTime(100);
        expect(prewakeTabId()).toBe("t1");
        leaveTabPill("t1");
        expect(prewakeTabId()).toBeNull();
        hoverTabPill("t1");
        leaveTabPill("t1");
        vi.advanceTimersByTime(200);
        expect(prewakeTabId()).toBeNull();
        pressTabPill("t1");
        expect(prewakeTabId()).toBe("t1");
    });
});

describe("the commands", () => {
    const pane = (visible: boolean, asleep: boolean, declined: string | null = null) => {
        const calls: string[] = [];
        const p: HibernatablePane & { calls: string[] } = {
            blockId: Math.random().toString(),
            calls,
            isVisible: () => visible,
            isAsleep: () => asleep,
            hiddenSince: () => (visible ? null : 0),
            tryHibernate: async (r) => {
                calls.push(`hibernate:${r}`);
                return declined;
            },
            wake: (w) => calls.push(`wake:${w}`),
        };
        return p;
    };

    it("hibernate only hidden, awake panes, and count only those that slept; wake only sleeping ones", async () => {
        const shown = pane(true, false);
        const hidden = pane(false, false);
        const busy = pane(false, false, "playing media");
        const sleeping = pane(false, true);
        const offs = [shown, hidden, busy, sleeping].map(registerHibernatablePane);
        expect(await hibernateHiddenBrowserPanes()).toBe(1);
        expect(hidden.calls).toEqual(["hibernate:manual"]);
        expect(busy.calls).toEqual(["hibernate:manual"]);
        expect(shown.calls).toEqual([]);
        expect(wakeAllBrowserPanes()).toBe(1);
        expect(sleeping.calls).toEqual(["wake:command"]);
        offs.forEach((off) => off());
    });
});

describe("when a hidden pane sleeps", () => {
    afterEach(() => {
        for (const k of Object.keys(settings)) delete settings[k];
    });

    it("waits 30 minutes, the setting's minutes, or less under memory pressure", () => {
        expect(hibernateAfterMs("normal")).toBe(30 * 60_000);
        expect(hibernateAfterMs("normal", 45)).toBe(45 * 60_000);
        expect(hibernateAfterMs("normal", 0)).toBe(30 * 60_000);
        expect(hibernateAfterMs("warn", 45)).toBe(5 * 60_000);
        expect(hibernateAfterMs("warn", 2)).toBe(2 * 60_000);
        expect(hibernateAfterMs("critical")).toBe(60_000);
    });

    it("never a local dev server's page", () => {
        expect(isLocalAddress("http://localhost:4600/")).toBe(true);
        expect(isLocalAddress("http://127.0.0.1:8080/x")).toBe(true);
        expect(isLocalAddress("http://[::1]:3000/")).toBe(true);
        expect(isLocalAddress("http://app.localhost/")).toBe(true);
        expect(isLocalAddress("https://github.com/")).toBe(false);
    });

    it("never a site the person kept awake, by host, subdomain or origin", () => {
        const sites = ["mail.google.com", "https://app.example.com"];
        expect(siteKeptAwake("https://mail.google.com/mail/u/0", sites)).toBe(true);
        expect(siteKeptAwake("https://x.mail.google.com/", sites)).toBe(true);
        expect(siteKeptAwake("https://app.example.com/a", sites)).toBe(true);
        expect(siteKeptAwake("http://app.example.com/a", sites)).toBe(false);
        expect(siteKeptAwake("https://google.com/", sites)).toBe(false);
        expect(siteKeptAwake("https://google.com/", undefined)).toBe(false);
    });

    const idle: PaneFacts = {
        url: "https://github.com/",
        keepAwake: false,
        driverAgent: false,
        bypass: false,
        attention: false,
        popupWindows: false,
        capturing: false,
        activeWhileHidden: false,
    };

    it("stays awake for what the window knows: kept awake, an agent, a banner, popups, capture, activity", () => {
        const now = 1_000_000_000;
        expect(awakeReason(idle, now)).toBeNull();
        expect(awakeReason({ ...idle, keepAwake: true }, now)).toBe("kept awake");
        expect(awakeReason({ ...idle, url: "about:blank" }, now)).toBe("no page");
        expect(awakeReason({ ...idle, driverAgent: true, lastAgentActionAt: now - 20 * 60_000 }, now)).toBe("an agent drives it");
        expect(awakeReason({ ...idle, bypass: true }, now)).toBe("auto-approve is on");
        expect(awakeReason({ ...idle, attention: true }, now)).toBe("waiting on the person");
        expect(awakeReason({ ...idle, popupWindows: true }, now)).toBe("popup windows open");
        expect(awakeReason({ ...idle, lastAgentActionAt: now - 60_000 }, now)).toBe("an agent used it recently");
        expect(awakeReason({ ...idle, lastAgentActionAt: now - 11 * 60_000 }, now)).toBeNull();
        expect(awakeReason({ ...idle, capturing: true }, now)).toBe("capturing media");
        expect(awakeReason({ ...idle, activeWhileHidden: true }, now)).toBe("active while hidden");
        expect(awakeReason(idle, now, ["github.com"])).toBe("site kept awake");
    });

    describe("a hidden page that is doing something on its own", () => {
        const T = 1_000_000;
        const github = "Releases · agentmuxai/agentmux";
        const icon = "https://github.com/favicon.ico";
        const watch = () => {
            const a = createPageActivity();
            a.rebase(github, icon, false, 0);
            return a;
        };

        it("is active once its title or icon takes a new value, and lapses after a while", () => {
            const a = watch();
            a.see("(2) Inbox", icon, false, T);
            expect(a.active(T)).toBe(true);
            expect(a.active(T + ACTIVE_LAPSE_MS - 1)).toBe(true);
            expect(a.active(T + ACTIVE_LAPSE_MS)).toBe(false);
            const b = watch();
            b.see(github, "https://github.com/other.ico", false, T);
            expect(b.active(T)).toBe(true);
        });

        it("is not active for the same title or icon announced again", () => {
            const a = watch();
            a.see(github, icon, false, T);
            a.see(github, icon, false, T + 60_000);
            expect(a.active(T + 60_000)).toBe(false);
        });

        it("is not active for the stand-ins a page shows before it has its own title and icon", () => {
            const a = watch();
            a.see("Browser", "", false, T);
            a.see(github, icon, false, T + 1000);
            expect(a.active(T + 1000)).toBe(false);
        });

        it("is not active for changes while its page loads, or just after", () => {
            const a = watch();
            a.see("example.com", icon, true, T);
            a.see("Example Domain", icon, true, T + 100);
            a.see("Example Domain", icon, false, T + 200);
            a.see("Example Domain!", icon, false, T + 200 + LOAD_SETTLE_MS - 1);
            expect(a.active(T + 300)).toBe(false);
            // After the load has settled, a change counts.
            a.see("Example Domain (1)", icon, false, T + 200 + LOAD_SETTLE_MS);
            expect(a.active(T + 200 + LOAD_SETTLE_MS)).toBe(true);
        });

        it("starts from what a pane that mounts hidden shows, and forgets when it is shown", () => {
            const a = createPageActivity();
            a.see(github, icon, false, T);
            expect(a.active(T)).toBe(false);
            a.see("(1) Releases", icon, false, T + 1000);
            expect(a.active(T + 1000)).toBe(true);
            a.rebase("(1) Releases", icon, false, T + 1500);
            expect(a.active(T + 2000)).toBe(false);
        });

        it("keeps the settling window of a page that finished loading while it was shown", () => {
            const a = createPageActivity();
            a.rebase("example.com", icon, true, T);
            // The load ends while shown, then the pane is hidden before the final title arrives.
            a.rebase("example.com", icon, false, T + 100);
            a.see("Example Domain", icon, false, T + 1000);
            expect(a.active(T + 1000)).toBe(false);
            a.see("Example Domain (1)", icon, false, T + 100 + LOAD_SETTLE_MS);
            expect(a.active(T + 100 + LOAD_SETTLE_MS)).toBe(true);
        });
    });

    it("counts a Browser call srv noted as the agent's use, though a read leaves no action in the log", () => {
        expect(lastAgentTouch(undefined, undefined)).toBeUndefined();
        expect(lastAgentTouch(1000, undefined)).toBe(1000);
        expect(lastAgentTouch(undefined, 2000)).toBe(2000);
        expect(lastAgentTouch(1000, 2000)).toBe(2000);
        expect(lastAgentTouch(3000, 2000)).toBe(3000);
        expect(lastAgentTouch(1000, "x")).toBe(1000);
        expect(lastAgentTouch(undefined, 0)).toBeUndefined();
        // An agent that only reads the pane keeps it awake through the grace period, then not.
        const now = 10 * 60 * 60_000;
        const driven = { ...idle, driverAgent: true };
        expect(awakeReason({ ...driven, lastAgentActionAt: lastAgentTouch(now - 3 * 60 * 60_000, now - 20 * 60_000) }, now)).toBe("an agent drives it");
        expect(awakeReason({ ...driven, lastAgentActionAt: lastAgentTouch(now - 3 * 60 * 60_000, now - 40 * 60_000) }, now)).toBeNull();
    });

    it("lets an agent's pane sleep once the agent has left it for the time any hidden pane waits", () => {
        const now = 10 * 60 * 60_000;
        const driven = { ...idle, driverAgent: true };
        expect(awakeReason({ ...driven, lastAgentActionAt: now - 29 * 60_000 }, now)).toBe("an agent drives it");
        expect(awakeReason({ ...driven, lastAgentActionAt: now - 30 * 60_000 }, now)).toBeNull();
        // Opened by an agent that has done nothing on it yet.
        expect(awakeReason(driven, now)).toBeNull();
        // The time follows `browser:hibernateafter`.
        settings["browser:hibernateafter"] = 60;
        expect(awakeReason({ ...driven, lastAgentActionAt: now - 45 * 60_000 }, now)).toBe("an agent drives it");
        delete settings["browser:hibernateafter"];
        // Auto-approve and a recent action still keep it awake.
        expect(awakeReason({ ...driven, bypass: true, lastAgentActionAt: now - 3 * 60 * 60_000 }, now)).toBe("auto-approve is on");
        expect(awakeReason({ ...idle, lastAgentActionAt: now - 5 * 60_000 }, now)).toBe("an agent used it recently");
    });

    describe("asking the page's beforeunload handlers", () => {
        const T = 5_000_000;
        const clean = { live: true, media: false, edited: false, beforeunload: false } as BrowserHibernationCheck;
        const asks = { ...clean, beforeunload: true } as BrowserHibernationCheck;

        it("keeps a 'prompts' answer until the pane is shown: a handler that ran once may be gone", () => {
            const m = createHandlerMemo();
            expect(m.mode(T)).toBe("unless_decided");
            expect(m.settle("unless_decided", asks, T)?.beforeunload).toBe(true);
            expect(m.mode(T + 30_000)).toBe("skip");
            expect(m.settle("skip", { ...clean }, T + 30_000)?.beforeunload).toBe(true);
            expect(m.mode(T + 10 * HANDLER_RECHECK_MS)).toBe("skip");
        });

        it("asks again after a while when the page could not be told, and keeps the pane awake meanwhile", () => {
            const m = createHandlerMemo();
            const unsure = { ...asks, beforeunload_unsure: true } as BrowserHibernationCheck;
            expect(m.settle("unless_decided", unsure, T)?.beforeunload).toBe(true);
            expect(m.mode(T + 30_000)).toBe("skip");
            expect(m.settle("skip", { ...clean }, T + 30_000)?.beforeunload).toBe(true);
            expect(m.mode(T + HANDLER_RECHECK_MS)).toBe("unless_decided");
            // The next look can find it does not prompt, and the pane may sleep.
            expect(m.settle("unless_decided", clean, T + HANDLER_RECHECK_MS)?.beforeunload).toBe(false);
            expect(m.settle("skip", { ...clean }, T + HANDLER_RECHECK_MS + 30_000)?.beforeunload).toBe(false);
        });

        it("keeps a 'no prompt' answer for a while, so a pane kept awake for another reason is not asked each tick", () => {
            const m = createHandlerMemo();
            m.settle("unless_decided", clean, T);
            expect(m.mode(T + 30_000)).toBe("skip");
            expect(m.settle("skip", { ...clean, devtools: true } as BrowserHibernationCheck, T + 30_000)?.beforeunload).toBe(false);
            expect(m.mode(T + HANDLER_RECHECK_MS - 1)).toBe("skip");
            expect(m.mode(T + HANDLER_RECHECK_MS)).toBe("unless_decided");
        });

        it("does not count handlers that did not run, and asks again once the pane has been shown", () => {
            const m = createHandlerMemo();
            m.settle("unless_decided", { ...clean, media: true } as BrowserHibernationCheck, T);
            expect(m.mode(T + 30_000)).toBe("unless_decided");
            m.settle("unless_decided", asks, T + 30_000);
            expect(m.mode(T + 60_000)).toBe("skip");
            m.forget();
            expect(m.mode(T + 60_000)).toBe("unless_decided");
        });

        it("passes a page that is not live through", () => {
            const m = createHandlerMemo();
            expect(m.settle("unless_decided", { live: false } as BrowserHibernationCheck, T)).toEqual({ live: false });
            expect(m.settle("skip", null, T)).toBeNull();
            expect(m.mode(T)).toBe("unless_decided");
        });
    });

    it("stays awake for what the page is doing, and when the page couldn't be checked", () => {
        expect(pageAwakeReason({ live: true })).toBeNull();
        expect(pageAwakeReason({ live: true, media: true })).toBe("playing media");
        expect(pageAwakeReason({ live: true, edited: true })).toBe("something typed into it");
        expect(pageAwakeReason({ live: true, beforeunload: true })).toBe("the page asks before leaving");
        expect(pageAwakeReason({ live: true, devtools: true })).toBe("DevTools open");
        expect(pageAwakeReason({ live: false })).toBe("no live page");
        expect(pageAwakeReason(null)).toBe("page not checked");
    });

    it("puts the oldest-hidden due panes to sleep, one per tick normally and more under pressure", async () => {
        const now = 10 * 60 * 60_000;
        const made: string[] = [];
        const hidden = (id: string, minutesAgo: number | null, visible = false) => ({
            blockId: id,
            isVisible: () => visible,
            isAsleep: () => false,
            hiddenSince: () => (minutesAgo == null ? null : now - minutesAgo * 60_000),
            tryHibernate: async (r: string) => {
                made.push(`${id}:${r}`);
                return null;
            },
            wake: () => {},
        });
        const offs = [hidden("a", 31), hidden("b", 120), hidden("c", 10), hidden("d", 6), hidden("v", null, true)].map(
            registerHibernatablePane
        );
        expect(await policyTick(now, async () => "normal")).toEqual(["b"]);
        expect(made).toEqual(["b:hidden"]);
        made.length = 0;
        expect(await policyTick(now, async () => "warn")).toEqual(["b", "a", "c"]);
        expect(made).toEqual(["b:pressure", "a:pressure", "c:pressure"]);
        made.length = 0;
        expect(await policyTick(now, async () => "critical")).toEqual(["b", "a", "c", "d"]);
        settings["browser:hibernate"] = false;
        expect(await policyTick(now, async () => "critical")).toEqual([]);
        offs.forEach((off) => off());
    });
});

describe("bySizeThenAge", () => {
    const pane = (blockId: string, hidden: number) => ({ blockId, hiddenSince: () => hidden });
    it("puts the biggest pages first, then those with no figure, each longest-hidden first", () => {
        const due = [pane("old-small", 1), pane("new-big", 9), pane("unknown-new", 8), pane("unknown-old", 2), pane("mid", 5)];
        const order = bySizeThenAge(due, { "old-small": 100, "new-big": 900, mid: 400 }).map((p) => p.blockId);
        expect(order).toEqual(["new-big", "mid", "old-small", "unknown-old", "unknown-new"]);
    });
    it("is longest-hidden first when nothing is known", () => {
        const due = [pane("b", 5), pane("a", 1), pane("c", 3)];
        expect(bySizeThenAge(due, {}).map((p) => p.blockId)).toEqual(["a", "c", "b"]);
    });
    it("does not change the list it was given", () => {
        const due = [pane("b", 5), pane("a", 1)];
        bySizeThenAge(due, { b: 1 });
        expect(due.map((p) => p.blockId)).toEqual(["b", "a"]);
    });
});

describe("policyTick under memory pressure", () => {
    const now = 10 * 60 * 60_000;
    const hidden = (id: string, minutesAgo: number, made: string[]) => ({
        blockId: id,
        isVisible: () => false,
        isAsleep: () => false,
        hiddenSince: () => now - minutesAgo * 60_000,
        tryHibernate: async (r: string) => {
            made.push(`${id}:${r}`);
            return null;
        },
        wake: () => {},
    });

    it("sleeps the biggest pages first when more are due than may sleep, and asks only about the due ones", async () => {
        const made: string[] = [];
        const asked: string[][] = [];
        // Warn lets three sleep a tick; five are due (hidden 6 minutes or more).
        const offs = [
            hidden("a", 60, made),
            hidden("b", 50, made),
            hidden("c", 40, made),
            hidden("d", 30, made),
            hidden("e", 20, made),
            hidden("young", 1, made),
        ].map(registerHibernatablePane);
        const memory = async (ids: string[]) => {
            asked.push(ids);
            return { a: 10, b: 20, c: 30, d: 400, e: 300, young: 9999 } as Record<string, number>;
        };
        expect(await policyTick(now, async () => "warn", memory)).toEqual(["d", "e", "c"]);
        expect(asked).toHaveLength(1);
        expect(asked[0].sort()).toEqual(["a", "b", "c", "d", "e"]);
        offs.forEach((off) => off());
    });

    it("keeps longest-hidden first when the host can't say, and never asks when few are due or the pressure is normal", async () => {
        const made: string[] = [];
        let asks = 0;
        const memory = async () => {
            asks++;
            return {};
        };
        const offs = [hidden("a", 60, made), hidden("b", 50, made), hidden("c", 40, made), hidden("d", 30, made)].map(
            registerHibernatablePane
        );
        expect(await policyTick(now, async () => "warn", memory)).toEqual(["a", "b", "c"]);
        expect(asks).toBe(1);
        expect(await policyTick(now, async () => "normal", memory)).toEqual(["a"]);
        expect(asks).toBe(1);
        // Three or fewer due under warn: all sleep, so the order is moot.
        offs.pop()?.();
        offs.shift()?.();
        asks = 0;
        expect(await policyTick(now, async () => "warn", memory)).toEqual(["b", "c"]);
        expect(asks).toBe(0);
        offs.forEach((off) => off());
    });
});

describe("createFreezeState", () => {
    it("clears the flag only once the host has answered the thaw", async () => {
        let answer!: (v: boolean) => void;
        const host = vi.fn(() => new Promise<boolean>((r) => (answer = r)));
        const f = createFreezeState(host);
        f.markFrozen();
        const done = f.thaw();
        expect(host).toHaveBeenCalledWith(false);
        expect(f.isFrozen()).toBe(true); // the host has not thawed it yet
        answer(true);
        await done;
        expect(f.isFrozen()).toBe(false);
    });
    it("asks the host once for thaws that overlap", async () => {
        const host = vi.fn(() => Promise.resolve(true));
        const f = createFreezeState(host);
        f.markFrozen();
        await Promise.all([f.thaw(), f.thaw()]);
        expect(host).toHaveBeenCalledTimes(1);
    });
    it("leaves a page frozen when the host's thaw failed, and tries again", async () => {
        const host = vi.fn().mockRejectedValueOnce(new Error("no")).mockResolvedValue(true);
        const f = createFreezeState(host);
        f.markFrozen();
        await f.thaw();
        expect(f.isFrozen()).toBe(true);
        await f.thaw();
        expect(f.isFrozen()).toBe(false);
        expect(host).toHaveBeenCalledTimes(2);
    });
    it("does not call the host for a page that is not frozen", async () => {
        const host = vi.fn(() => Promise.resolve(true));
        await createFreezeState(host).thaw();
        expect(host).not.toHaveBeenCalled();
    });
});

describe("wakeEndsNow", () => {
    it("ends at once when the pane is shown or the wake was asked for", () => {
        expect(wakeEndsNow(true, "intent")).toBe(true);
        expect(wakeEndsNow(true, "show")).toBe(true);
        expect(wakeEndsNow(false, "command")).toBe(true);
        expect(wakeEndsNow(false, "agent")).toBe(true);
    });
    it("holds a hover's wake that finished while the pane is still hidden", () => {
        expect(wakeEndsNow(false, "intent")).toBe(false);
        expect(wakeEndsNow(false, "show")).toBe(false);
    });
});

describe("afterDiscard", () => {
    it("keeps a page the engine discarded in place, and closes it otherwise", () => {
        expect(afterDiscard({ supported: true, ok: true }, true)).toBe("kept");
        expect(afterDiscard({ supported: true, ok: false }, true)).toBe("close");
        expect(afterDiscard({ supported: false, ok: false }, true)).toBe("close");
        expect(afterDiscard(null, true)).toBe("close");
    });
    it("loads back a page discarded while it was being woken, and leaves an undiscarded one alone", () => {
        expect(afterDiscard({ supported: true, ok: true }, false)).toBe("load");
        expect(afterDiscard({ supported: true, ok: false }, false)).toBe("nothing");
        expect(afterDiscard(undefined, false)).toBe("nothing");
    });
});

describe("normalizeKeepAwakeSite", () => {
    it("keeps a host, which covers its subdomains", () => {
        expect(normalizeKeepAwakeSite("  Example.COM ")).toBe("example.com");
        expect(siteKeptAwake("https://mail.example.com/inbox", [normalizeKeepAwakeSite("example.com")])).toBe(true);
    });
    it("cuts a pasted address to its host, and keeps an origin when a scheme is given", () => {
        expect(normalizeKeepAwakeSite("example.com/path?q=1")).toBe("example.com");
        expect(normalizeKeepAwakeSite("https://App.example.com:8443/x")).toBe("https://app.example.com:8443");
        expect(siteKeptAwake("https://app.example.com:8443/y", [normalizeKeepAwakeSite("https://app.example.com:8443/x")])).toBe(true);
        expect(siteKeptAwake("https://other.example.com/", [normalizeKeepAwakeSite("https://app.example.com:8443/x")])).toBe(false);
    });
    it("refuses what names no site", () => {
        expect(normalizeKeepAwakeSite("")).toBeNull();
        expect(normalizeKeepAwakeSite("hello")).toBeNull();
        expect(normalizeKeepAwakeSite("data:text/html,hi")).toBeNull();
        expect(normalizeKeepAwakeSite("localhost")).toBe("localhost");
    });
});

describe("freezing hidden panes", () => {
    it("freezes panes hidden 5 minutes that aren't asleep or frozen, those that declined to sleep too", async () => {
        settings["browser:hibernate"] = true;
        const now = 10 * 60 * 60_000;
        const frozeCalls: string[] = [];
        const pane = (id: string, minutesAgo: number, opts: { frozen?: boolean; declineSleep?: boolean } = {}) => ({
            blockId: id,
            isVisible: () => false,
            isAsleep: () => false,
            hiddenSince: () => now - minutesAgo * 60_000,
            tryHibernate: async () => (opts.declineSleep ? "an agent is driving it" : null),
            wake: () => {},
            isFrozen: () => !!opts.frozen,
            tryFreeze: async () => {
                frozeCalls.push(id);
                return null;
            },
        });
        const offs = [
            pane("short", 3),
            pane("due-freeze", 6),
            pane("already", 8, { frozen: true }),
            pane("kept-awake", 45, { declineSleep: true }),
        ].map(registerHibernatablePane);
        const slept = await policyTick(now, async () => "normal");
        expect(slept).toEqual([]);
        expect(frozeCalls).toEqual(["kept-awake", "due-freeze"]);
        offs.forEach((off) => off());
    });

    it("freezes at most FREEZE_PER_TICK panes a tick, and only for what a frozen page would break", async () => {
        settings["browser:hibernate"] = true;
        const now = 10 * 60 * 60_000;
        let n = 0;
        const offs = Array.from({ length: FREEZE_PER_TICK + 2 }, (_, i) => ({
            blockId: `p${i}`,
            isVisible: () => false,
            isAsleep: () => false,
            hiddenSince: () => now - 10 * 60_000,
            tryHibernate: async () => "kept",
            wake: () => {},
            isFrozen: () => false,
            tryFreeze: async () => {
                n++;
                return null;
            },
        })).map(registerHibernatablePane);
        await policyTick(now, async () => "normal");
        expect(n).toBe(FREEZE_PER_TICK);
        offs.forEach((off) => off());
        expect(freezeAwakeReason({ live: true, edited: true, beforeunload: true })).toBeNull();
        expect(freezeAwakeReason({ live: true, media: true })).toBe("playing media");
        expect(freezeAwakeReason({ live: true, devtools: true })).toBe("DevTools open");
    });
});

describe("engineSupportsFreeze", () => {
    afterEach(() => resetEngineSupportsFreeze());
    it("asks again after a while when the answer was no, and keeps a yes", async () => {
        const answers = [{ supported: false }, { supported: true }];
        const ask = vi.fn(async () => answers.shift()!);
        expect(await engineSupportsFreeze(ask, 1000)).toBe(false);
        expect(await engineSupportsFreeze(ask, 1000 + ENGINE_NO_RECHECK_MS - 1)).toBe(false);
        expect(ask).toHaveBeenCalledTimes(1);
        expect(await engineSupportsFreeze(ask, 1000 + ENGINE_NO_RECHECK_MS)).toBe(true);
        expect(await engineSupportsFreeze(ask, 1000 + 10 * ENGINE_NO_RECHECK_MS)).toBe(true);
        expect(ask).toHaveBeenCalledTimes(2);
    });
    it("asks once, and says yes only for an engine with the Discard API", async () => {
        let asked = 0;
        const ask = async () => {
            asked++;
            return { supported: true, ok: false };
        };
        expect(await engineSupportsFreeze(ask)).toBe(true);
        expect(await engineSupportsFreeze(ask)).toBe(true);
        expect(asked).toBe(1);
    });
});
