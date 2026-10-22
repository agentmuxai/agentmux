// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
const settings: Record<string, unknown> = {};
vi.mock("@/store/global", () => ({ getSettingsKeyAtom: (k: string) => () => settings[k] }));
vi.mock("@/app/store/app-api", () => ({ getApi: () => undefined }));

import {
    awakeReason,
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
    wakeAllBrowserPanes,
    type HibernatablePane,
    type PaneFacts,
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
        });
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
        expect(awakeReason({ ...idle, driverAgent: true }, now)).toBe("an agent drives it");
        expect(awakeReason({ ...idle, bypass: true }, now)).toBe("auto-approve is on");
        expect(awakeReason({ ...idle, attention: true }, now)).toBe("waiting on the person");
        expect(awakeReason({ ...idle, popupWindows: true }, now)).toBe("popup windows open");
        expect(awakeReason({ ...idle, lastAgentActionAt: now - 60_000 }, now)).toBe("an agent used it recently");
        expect(awakeReason({ ...idle, lastAgentActionAt: now - 11 * 60_000 }, now)).toBeNull();
        expect(awakeReason({ ...idle, capturing: true }, now)).toBe("capturing media");
        expect(awakeReason({ ...idle, activeWhileHidden: true }, now)).toBe("active while hidden");
        expect(awakeReason(idle, now, ["github.com"])).toBe("site kept awake");
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
