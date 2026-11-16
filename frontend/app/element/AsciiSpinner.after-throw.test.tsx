// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * #47754: Swarm rows showing "working" with their mark frozen, each on a different frame, until the window was
 * restarted. Solid drops the queued computations of an update when one of them throws (the app logs a
 * "Stale read from <Show>" now and then) and never queues them again, so a spinner that read the shared tick
 * and was behind the thrower in that tick stopped for good. The marks must not depend on that: one failing
 * subscriber of the tick, once, leaves every mark stepping.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createEffect } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AsciiSpinner, SPINNER_FRAME_MS } from "./AsciiSpinner";
import { sharedTick } from "./shared-ticker";

describe("the shared ticker's marks after a subscriber of the tick throws", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    let thrown = 0;

    /** A bystander that reads the tick, as an animation does, and fails once when told to. */
    function Bystander(props: { fail: () => boolean }) {
        createEffect(() => {
            sharedTick();
            if (props.fail()) {
                thrown++;
                throw new Error("Stale read from <Show>.");
            }
        });
        return null;
    }

    it("every spinner keeps stepping once the failing tick has passed", () => {
        let fail = false;
        const view = render(() => (
            <div>
                <AsciiSpinner />
                <Bystander fail={() => fail} />
                <AsciiSpinner />
                <AsciiSpinner />
            </div>
        ));
        const marks = () => [...view.container.querySelectorAll("span")].map((s) => s.textContent);
        vi.advanceTimersByTime(SPINNER_FRAME_MS * 2);

        fail = true;
        try {
            vi.advanceTimersByTime(SPINNER_FRAME_MS);
        } catch {
            // the thrown error surfaces from the interval, as an uncaught error does in the window
        }
        fail = false;
        expect(thrown).toBeGreaterThan(0);

        const before = marks();
        vi.advanceTimersByTime(SPINNER_FRAME_MS);
        const after = marks();
        expect(after).toHaveLength(3);
        after.forEach((frame, i) => expect(frame, `spinner ${i}`).not.toBe(before[i]));
        // And all three show the one frame of the one clock.
        expect(new Set(after).size).toBe(1);
    });
});
