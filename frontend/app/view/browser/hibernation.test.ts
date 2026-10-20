// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import {
    etaLabel,
    hibernateHiddenBrowserPanes,
    hoverTabPill,
    isOverdue,
    leaveTabPill,
    pressTabPill,
    prewakeTabId,
    readSleep,
    registerHibernatablePane,
    sleepName,
    sleepReasonText,
    wakeAllBrowserPanes,
    type HibernatablePane,
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
    const pane = (visible: boolean, asleep: boolean): HibernatablePane & { calls: string[] } => {
        const calls: string[] = [];
        return {
            blockId: Math.random().toString(),
            calls,
            isVisible: () => visible,
            isAsleep: () => asleep,
            hibernate: (r) => calls.push(`hibernate:${r}`),
            wake: (w) => calls.push(`wake:${w}`),
        };
    };

    it("hibernate only hidden, awake panes; wake only sleeping ones", () => {
        const shown = pane(true, false);
        const hidden = pane(false, false);
        const sleeping = pane(false, true);
        const offs = [shown, hidden, sleeping].map(registerHibernatablePane);
        expect(hibernateHiddenBrowserPanes()).toBe(1);
        expect(hidden.calls).toEqual(["hibernate:manual"]);
        expect(shown.calls).toEqual([]);
        expect(wakeAllBrowserPanes()).toBe(1);
        expect(sleeping.calls).toEqual(["wake:command"]);
        offs.forEach((off) => off());
    });
});
