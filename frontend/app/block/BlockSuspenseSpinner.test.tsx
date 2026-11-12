// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A block's mark for a pending read in its view (BlockSuspenseSpinner.tsx): logged from the moment the read
 * is pending, drawn only once the read has taken a while, and gone with the read.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal, Show } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BlockSuspenseSpinner, PENDING_MARK_AFTER_MS } from "./BlockSuspenseSpinner";

let info: ReturnType<typeof vi.spyOn>;
const lines = (): string[] => info.mock.calls.filter((c) => c[0] === "[pane-cover]").map((c) => String(c[1]));

beforeEach(() => {
    vi.useFakeTimers();
    info = vi.spyOn(console, "info").mockImplementation(() => {});
});

afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe("a block's mark for a pending read", () => {
    it("is logged at once and drawn only after the wait", () => {
        const { container } = render(() => <BlockSuspenseSpinner blockId="0123456789" agent={() => "Agent2"} />);
        expect(lines()).toEqual(['block=0123456 agent="Agent2" suspense-fallback shown']);
        expect(container.querySelector(".brain-spinner")).toBeNull();
        vi.advanceTimersByTime(PENDING_MARK_AFTER_MS - 1);
        expect(container.querySelector(".brain-spinner")).toBeNull();
        vi.advanceTimersByTime(1);
        expect(container.querySelector(".brain-spinner")).not.toBeNull();
    });

    it("is never drawn for a read that comes back before the wait is over, and its time is still logged", () => {
        const [pending, setPending] = createSignal(true);
        const { container } = render(() => (
            <Show when={pending()}>
                <BlockSuspenseSpinner blockId="0123456789" agent={() => undefined} />
            </Show>
        ));
        vi.advanceTimersByTime(120);
        setPending(false);
        vi.advanceTimersByTime(PENDING_MARK_AFTER_MS);
        expect(container.querySelector(".brain-spinner")).toBeNull();
        expect(lines()).toEqual(["block=0123456 suspense-fallback shown", "block=0123456 suspense-fallback cleared after=120ms"]);
    });
});
