// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PaneReadinessPhase } from "@/app/store/pane-readiness";
import {
    agentNameOf,
    coverLogLine,
    logCoverPhases,
    logShowCover,
    logWhileShown,
    SETTLED_AFTER_MS,
    shownLine,
    SLOW_COVER_MS,
    stillCoveredLine,
    unmountLine,
} from "./pane-cover-log";

afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
});

/** The `[pane-cover]` lines written so far. */
const coverLines = (info: ReturnType<typeof vi.spyOn>): string[] =>
    info.mock.calls.filter((c) => c[0] === "[pane-cover]").map((c) => String(c[1]));

describe("coverLogLine", () => {
    it("names the block, the change, the source and the time since mount", () => {
        expect(coverLogLine("0123456789", "assembling", "revealing", "controller", 1102.6)).toBe(
            "block=0123456 assembling→revealing src=controller t=1103ms"
        );
    });

    it("says mount for the first phase", () => {
        expect(coverLogLine("abcdefgh", undefined, "assembling", "controller", 0)).toBe(
            "block=abcdefg mount→assembling src=controller t=0ms"
        );
    });

    it("names the agent when it has a name", () => {
        expect(coverLogLine("0123456789", "live", "assembling", "re-cover", 5, "Manoz")).toBe(
            'block=0123456 agent="Manoz" live→assembling src=re-cover t=5ms'
        );
    });
});

describe("the other lines", () => {
    it("says what a cover waits on, for how long, and who holds it", () => {
        expect(stillCoveredLine("0123456789", "controller", "assembling", 1000.4, ["content", "history"], "Lazo")).toBe(
            'block=0123456 agent="Lazo" still-covered src=controller phase=assembling for=1000ms pending=content,history'
        );
        expect(stillCoveredLine("0123456789", "re-cover", "assembling", 2000, [])).toBe(
            "block=0123456 still-covered src=re-cover phase=assembling for=2000ms pending=none"
        );
    });

    it("says what a pane was doing as its view went away", () => {
        expect(unmountLine("0123456789", { controller: "assembling", reCover: "live" }, 12.4, "block=gone")).toBe(
            "block=0123456 unmount controller=assembling re-cover=live after=12ms block=gone"
        );
    });
});

describe("logCoverPhases", () => {
    it("writes one [pane-cover] line per change of phase, each under its own source", () => {
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        const [controller, setController] = createSignal<PaneReadinessPhase>("assembling");
        const [reCover, setReCover] = createSignal<PaneReadinessPhase>("live");
        const dispose = createRoot((d) => {
            logCoverPhases("0123456789", { controller, reCover });
            return d;
        });
        setController("revealing");
        setController("live"); // the controller's own last step is not a re-cover
        setReCover("assembling"); // a subagent "started" re-covers the pane
        setReCover("revealing");
        setReCover("live");
        const changes = coverLines(info).filter((l) => l.includes(" src=") && !l.includes("still-covered"));
        dispose();
        expect(changes.map((l) => l.replace(/ t=\d+ms$/, ""))).toEqual([
            "block=0123456 mount→assembling src=controller",
            "block=0123456 assembling→revealing src=controller",
            "block=0123456 revealing→live src=controller",
            "block=0123456 live→assembling src=re-cover",
            "block=0123456 assembling→revealing src=re-cover",
            "block=0123456 revealing→live src=re-cover",
        ]);
    });

    it("writes still-covered once a second while a cover is up, and stops when it lifts", () => {
        vi.useFakeTimers();
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        const [controller, setController] = createSignal<PaneReadinessPhase>("assembling");
        const dispose = createRoot((d) => {
            logCoverPhases("0123456789", { controller, reCover: () => "live" }, { agent: () => "Lazo", pending: () => ["content"] });
            return d;
        });
        vi.advanceTimersByTime(3000);
        const stills = coverLines(info).filter((l) => l.includes("still-covered"));
        expect(stills).toHaveLength(3);
        expect(stills[2]).toBe('block=0123456 agent="Lazo" still-covered src=controller phase=assembling for=3000ms pending=content');
        setController("live");
        info.mockClear();
        vi.advanceTimersByTime(5000);
        expect(coverLines(info).filter((l) => l.includes("still-covered"))).toEqual([]);
        dispose();
    });

    it("writes a settled line after a slow cover lifts, and none after a quick one", () => {
        vi.useFakeTimers();
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        const [slow, setSlow] = createSignal<PaneReadinessPhase>("assembling");
        const [quick, setQuick] = createSignal<PaneReadinessPhase>("assembling");
        const dispose = createRoot((d) => {
            logCoverPhases("0123456789", { controller: slow, reCover: () => "live" }, { root: () => null });
            logCoverPhases("abcdefghij", { controller: quick, reCover: () => "live" }, { root: () => null });
            return d;
        });
        vi.advanceTimersByTime(400);
        setQuick("live");
        vi.advanceTimersByTime(SLOW_COVER_MS - 400 + 500);
        setSlow("live");
        info.mockClear();
        vi.advanceTimersByTime(SETTLED_AFTER_MS + 10);
        const settled = coverLines(info).filter((l) => l.includes(" settled "));
        dispose();
        expect(settled).toEqual(["block=0123456 settled after-cover=3500ms root=missing"]);
    });

    it("times a cover from when it went up, across its phase changes", () => {
        vi.useFakeTimers();
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        const [controller, setController] = createSignal<PaneReadinessPhase>("assembling");
        const dispose = createRoot((d) => {
            logCoverPhases("0123456789", { controller, reCover: () => "live" }, { root: () => null });
            return d;
        });
        vi.advanceTimersByTime(2500);
        setController("revealing");
        vi.advanceTimersByTime(1500);
        setController("live");
        const stills = coverLines(info).filter((l) => l.includes("still-covered"));
        expect(stills.map((l) => l.replace(/^.* for=(\d+)ms.*$/, "$1"))).toEqual(["1000", "2000", "3000", "4000"]);
        expect(stills[3]).toContain("phase=revealing");
        info.mockClear();
        vi.advanceTimersByTime(SETTLED_AFTER_MS + 10);
        expect(coverLines(info)).toEqual(["block=0123456 settled after-cover=4000ms root=missing"]);
        dispose();
    });

    it("writes an unmount line when the pane's view goes away, with why", () => {
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        const dispose = createRoot((d) => {
            logCoverPhases(
                "0123456789",
                { controller: () => "live", reCover: () => "live" },
                { agent: () => "Manoz", why: () => "block=gone parked=false" }
            );
            return d;
        });
        info.mockClear();
        dispose();
        expect(coverLines(info).map((l) => l.replace(/ after=\d+ms/, " after=Nms"))).toEqual([
            'block=0123456 agent="Manoz" unmount controller=live re-cover=live after=Nms block=gone parked=false',
        ]);
    });
});

describe("what else can be in front of a pane", () => {
    it("names the thing, the state and the time", () => {
        expect(shownLine("0123456789", "suspense-fallback", "shown", 0, "Agent2")).toBe('block=0123456 agent="Agent2" suspense-fallback shown');
        expect(shownLine("0123456789", "show-cover", "still-shown", 2000.4)).toBe("block=0123456 show-cover still-shown for=2000ms");
        expect(shownLine("0123456789", "show-cover", "cleared", 3100)).toBe("block=0123456 show-cover cleared after=3100ms");
    });

    it("reads the agent's name from the block's meta, when it is a string", () => {
        expect(agentNameOf({ meta: { agentName: "Agent2" } })).toBe("Agent2");
        expect(agentNameOf({ meta: { agentName: 7 } })).toBeUndefined();
        expect(agentNameOf(undefined)).toBeUndefined();
    });

    it("logWhileShown writes shown, a line a second, and cleared with the time when its scope ends", () => {
        vi.useFakeTimers();
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        const dispose = createRoot((d) => {
            logWhileShown("0123456789", "suspense-fallback", () => "Agent2");
            return d;
        });
        vi.advanceTimersByTime(2500);
        dispose();
        expect(coverLines(info)).toEqual([
            'block=0123456 agent="Agent2" suspense-fallback shown',
            'block=0123456 agent="Agent2" suspense-fallback still-shown for=1000ms',
            'block=0123456 agent="Agent2" suspense-fallback still-shown for=2000ms',
            'block=0123456 agent="Agent2" suspense-fallback cleared after=2500ms',
        ]);
        info.mockClear();
        vi.advanceTimersByTime(5000);
        expect(coverLines(info)).toEqual([]);
    });

    it("logShowCover keeps one timing when the block it reads the agent from changes meanwhile", () => {
        vi.useFakeTimers();
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        const [name, setName] = createSignal<string | undefined>(undefined);
        const dispose = createRoot((d) => {
            logShowCover("0123456789", () => true, name);
            return d;
        });
        vi.advanceTimersByTime(1000);
        setName("Agent2");
        vi.advanceTimersByTime(1000);
        dispose();
        expect(coverLines(info)).toEqual([
            "block=0123456 show-cover shown",
            "block=0123456 show-cover still-shown for=1000ms",
            'block=0123456 agent="Agent2" show-cover still-shown for=2000ms',
            'block=0123456 agent="Agent2" show-cover cleared after=2000ms',
        ]);
    });

    it("logShowCover logs each time the show-cover goes up, once, and says nothing without a block id", () => {
        vi.useFakeTimers();
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        const [up, setUp] = createSignal(false);
        const dispose = createRoot((d) => {
            logShowCover("0123456789", up);
            logShowCover(undefined, up);
            return d;
        });
        expect(coverLines(info)).toEqual([]);
        setUp(true);
        vi.advanceTimersByTime(1000);
        setUp(false);
        setUp(true);
        setUp(false);
        dispose();
        expect(coverLines(info)).toEqual([
            "block=0123456 show-cover shown",
            "block=0123456 show-cover still-shown for=1000ms",
            "block=0123456 show-cover cleared after=1000ms",
            "block=0123456 show-cover shown",
            "block=0123456 show-cover cleared after=0ms",
        ]);
    });
});
