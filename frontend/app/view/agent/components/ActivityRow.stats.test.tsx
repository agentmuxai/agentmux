// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The collapsed activity row: the whole summary on the left, the stats on the right, and no
 * stray last line of output in between.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ActivityRow } from "./ActivityRow";
import type { PinnedActivity } from "../../../agent-model/activity-types";

afterEach(cleanup);

const START = 1_000_000;

function row(over: Partial<PinnedActivity> = {}) {
    const activity: PinnedActivity = { id: "a1", kind: "tool", title: "Restart my dev build with the new settings", status: "running", startedAt: START, canStop: true, ...over };
    const [a] = createSignal<PinnedActivity | undefined>(activity);
    const [expanded] = createSignal(false);
    const [leaving] = createSignal(false);
    return render(() => <ActivityRow activity={a} expanded={expanded} leaving={leaving} onToggle={() => {}} onStop={() => {}} onDismiss={() => {}} />);
}

describe("ActivityRow stats", () => {
    it("shows the whole summary, and the elapsed time among the stats at the right", () => {
        vi.useFakeTimers();
        vi.setSystemTime(START + 404_000);
        const { container } = row();
        expect(container.querySelector(".agent-activity-title")?.textContent).toBe("Restart my dev build with the new settings");
        expect(container.querySelector(".agent-activity-stats .agent-activity-elapsed")?.textContent).toBe("6m 44s");
        vi.useRealTimers();
    });

    it("does not show the last line of a tool's output on the row", () => {
        const tool = { log: { chunks: [{ kind: "stdout", content: "Tests 267 passed (267)" }] }, params: {} } as unknown as PinnedActivity["tool"];
        const { container } = row({ tool, title: "Shrink the call to one test run" });
        expect(container.textContent).not.toContain("Tests 267 passed");
        expect(container.querySelector(".agent-activity-tail")).toBeNull();
    });
});
