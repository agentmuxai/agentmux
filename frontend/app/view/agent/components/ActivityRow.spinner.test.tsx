// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A running tool row in the dock shows the rotating slash to the left of its short phrase; a finished one
 * shows its result glyph, and a row of another kind keeps its own sigil.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";

import type { PinnedActivity } from "../activity/types";
import { ActivityRow } from "./ActivityRow";

afterEach(cleanup);

const activity = (over: Partial<PinnedActivity> = {}): PinnedActivity => ({
    id: "t1",
    kind: "tool",
    title: "Wait for v0.60.10 to publish",
    status: "running",
    startedAt: Date.now(),
    canStop: false,
    ...over,
});

const mount = (a: PinnedActivity) => {
    const [act] = createSignal<PinnedActivity | undefined>(a);
    const [no] = createSignal(false);
    return render(() => <ActivityRow activity={act} expanded={no} leaving={no} onToggle={() => {}} onStop={() => {}} onDismiss={() => {}} />);
};

describe("ActivityRow sigil", () => {
    it("a running tool shows the spinner just before its phrase", () => {
        const { container } = mount(activity());
        const summary = container.querySelector(".agent-activity-summary")!;
        const [first, second] = [...summary.children];
        expect(first.classList.contains("agent-activity-sigil")).toBe(true);
        expect(first.querySelector(".agent-activity-spinner")).not.toBeNull();
        expect(second.textContent).toBe("Wait for v0.60.10 to publish");
        expect(first.textContent).not.toContain("$");
    });

    it("a finished tool shows its result glyph, not the spinner", () => {
        const { container } = mount(activity({ status: "done" }));
        expect(container.querySelector(".agent-activity-spinner")).toBeNull();
        expect(container.querySelector(".agent-activity-sigil")?.textContent).toBe("✓");
    });

    it("a shell keeps its own sigil", () => {
        const { container } = mount(activity({ kind: "shell" }));
        expect(container.querySelector(".agent-activity-spinner")).toBeNull();
        expect(container.querySelector(".agent-activity-sigil")?.textContent).toBe("⟩");
    });
});
