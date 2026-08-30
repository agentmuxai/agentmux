// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The "~Ns left" countdown actually reaching the DOM.
 *
 * The rest of this feature is covered by pure-function tests
 * (`sleep-detect.test.ts`, `tool-adapter.test.ts`), which prove the number is
 * computed correctly — but not that anything renders it. This file closes
 * that gap deterministically, which is a better artifact than eyeballing a
 * screenshot once: the rendered output is a function of the activity's props,
 * so it can be asserted rather than observed.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ActivityRow } from "./ActivityRow";
import type { ActivityStatus, PinnedActivity } from "../activity/types";

afterEach(cleanup);

const START = 1_000_000;

function sleepActivity(over: Partial<PinnedActivity> = {}): PinnedActivity {
    return {
        id: "s1",
        kind: "tool",
        title: "sleep 300",
        status: "running",
        startedAt: START,
        canStop: false,
        sleepMs: 300_000,
        ...over,
    };
}

function renderRow(activity: PinnedActivity) {
    const [a] = createSignal<PinnedActivity | undefined>(activity);
    const [expanded] = createSignal(false);
    const [leaving] = createSignal(false);
    return render(() => (
        <ActivityRow
            activity={a}
            expanded={expanded}
            leaving={leaving}
            onToggle={() => {}}
            onStop={() => {}}
            onDismiss={() => {}}
        />
    ));
}

const countdownText = (c: HTMLElement) => c.querySelector(".agent-activity-remaining")?.textContent ?? null;

describe("ActivityRow — sleep countdown", () => {
    it("renders the remaining time for a running whole-command sleep", () => {
        vi.useFakeTimers();
        vi.setSystemTime(START + 40_000); // 40s into a 300s sleep
        const { container } = renderRow(sleepActivity());
        expect(countdownText(container)).toBe("~260s left");
        vi.useRealTimers();
    });

    it("counts down as time passes", () => {
        vi.useFakeTimers();
        vi.setSystemTime(START);
        const { container } = renderRow(sleepActivity());
        expect(countdownText(container)).toBe("~300s left");
        // useTick(1000) drives the recompute. advanceTimersByTime moves the
        // mocked clock as well as firing timers, so it alone is the elapsed
        // time — adding a setSystemTime on top would double-count it.
        vi.advanceTimersByTime(5_000);
        expect(countdownText(container)).toBe("~295s left");
        vi.useRealTimers();
    });

    /** The process is reaped slightly after its own deadline, so the last tick
     *  before ToolEnd must not render a negative number. */
    it("clamps at zero rather than going negative past the deadline", () => {
        vi.useFakeTimers();
        vi.setSystemTime(START + 305_000); // 5s past a 300s sleep
        const { container } = renderRow(sleepActivity());
        expect(countdownText(container)).toBe("~0s left");
        vi.useRealTimers();
    });

    it.each<ActivityStatus>(["done", "error", "stopped"])(
        "shows no countdown once terminal (%s) — the final elapsed is the reading",
        (status) => {
            vi.useFakeTimers();
            vi.setSystemTime(START + 40_000);
            const { container } = renderRow(sleepActivity({ status, endedAt: START + 30_000 }));
            expect(countdownText(container)).toBeNull();
            vi.useRealTimers();
        },
    );

    /** Every other activity kind — and every compound sleep — has no knowable
     *  remaining time, so the element must be absent entirely rather than
     *  rendering an empty or guessed value. */
    it("shows no countdown for an activity with no sleepMs", () => {
        vi.useFakeTimers();
        vi.setSystemTime(START + 40_000);
        const { container } = renderRow(sleepActivity({ sleepMs: undefined, title: "cargo test" }));
        expect(countdownText(container)).toBeNull();
        // …while the ordinary elapsed clock still renders, so this asserts the
        // countdown's absence, not the whole row failing to mount.
        expect(container.querySelector(".agent-activity-elapsed")).not.toBeNull();
        vi.useRealTimers();
    });

    it("renders the countdown alongside the elapsed clock, not instead of it", () => {
        vi.useFakeTimers();
        vi.setSystemTime(START + 40_000);
        const { container } = renderRow(sleepActivity());
        expect(container.querySelector(".agent-activity-elapsed")?.textContent).toBe("[0:40]");
        expect(countdownText(container)).toBe("~260s left");
        vi.useRealTimers();
    });
});
