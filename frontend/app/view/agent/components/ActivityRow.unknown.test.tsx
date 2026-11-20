// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An unknown row (a registry row nothing has vouched for in hours) has no clock. A stopped one read "0s" for a call
 * that may still be running; the row says how long ago anything last heard of it, and keeps counting.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { formatElapsedCompact } from "@/util/format-time";
import { ActivityRow } from "./ActivityRow";
import type { PinnedActivity } from "../../../agent-model/activity-types";

afterEach(cleanup);

const LAST_SEEN = 1_000_000;
const HOUR = 60 * 60 * 1000;

function unknownRow(over: Partial<PinnedActivity> = {}): PinnedActivity {
    return {
        id: "bg-1",
        kind: "tool",
        title: "Bash",
        status: "unknown",
        // Started and last seen in the same moment: the stopped clock the report showed as "0s".
        startedAt: LAST_SEEN,
        endedAt: LAST_SEEN,
        unknownSince: LAST_SEEN + 2 * HOUR,
        canStop: false,
        ...over,
    };
}

function renderRow(activity: PinnedActivity) {
    const [a] = createSignal<PinnedActivity | undefined>(activity);
    return render(() => (
        <ActivityRow activity={a} expanded={() => false} leaving={() => false} onToggle={() => {}} onStop={() => {}} onDismiss={() => {}} />
    ));
}

const elapsedText = (c: HTMLElement) => c.querySelector(".agent-activity-elapsed")?.textContent ?? null;

describe("ActivityRow, an unknown row", () => {
    it("says how long ago it was last seen, not a stopped 0s", () => {
        vi.useFakeTimers();
        vi.setSystemTime(LAST_SEEN + 2 * HOUR + 60_000);
        const { container } = renderRow(unknownRow());
        expect(elapsedText(container)).toBe(`seen ${formatElapsedCompact(2 * HOUR + 60_000)} ago`);
        expect(elapsedText(container)).not.toBe("0s");
        vi.useRealTimers();
    });

    it("keeps counting as time passes", () => {
        vi.useFakeTimers();
        vi.setSystemTime(LAST_SEEN + 2 * HOUR);
        const { container } = renderRow(unknownRow());
        const before = elapsedText(container);
        vi.advanceTimersByTime(5 * 60_000);
        expect(elapsedText(container)).toBe(`seen ${formatElapsedCompact(2 * HOUR + 5 * 60_000)} ago`);
        expect(elapsedText(container)).not.toBe(before);
        vi.useRealTimers();
    });

    it("leaves a finished row's figure alone: its elapsed is how long it ran", () => {
        vi.useFakeTimers();
        vi.setSystemTime(LAST_SEEN + HOUR);
        const { container } = renderRow(unknownRow({ status: "done", startedAt: LAST_SEEN - 90_000, endedAt: LAST_SEEN }));
        expect(elapsedText(container)).toBe(formatElapsedCompact(90_000));
        vi.useRealTimers();
    });
});
