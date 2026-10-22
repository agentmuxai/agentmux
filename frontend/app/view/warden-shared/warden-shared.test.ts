// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A Warden section polls only while it is shown (pollWhileVisible). */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { pollWhileVisible, WARDEN_REFRESH_MS } from "./warden-shared";

describe("pollWhileVisible", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("refreshes at once and every interval while shown, not while hidden, and at once when shown again", () => {
        const refresh = vi.fn();
        const [visible, setVisible] = createSignal(true);
        const dispose = createRoot((d) => {
            pollWhileVisible(refresh, visible);
            return d;
        });
        expect(refresh).toHaveBeenCalledTimes(1);
        vi.advanceTimersByTime(WARDEN_REFRESH_MS * 2);
        expect(refresh).toHaveBeenCalledTimes(3);
        setVisible(false);
        vi.advanceTimersByTime(WARDEN_REFRESH_MS * 5);
        expect(refresh).toHaveBeenCalledTimes(3);
        setVisible(true);
        expect(refresh).toHaveBeenCalledTimes(4);
        dispose();
        vi.advanceTimersByTime(WARDEN_REFRESH_MS * 5);
        expect(refresh).toHaveBeenCalledTimes(4);
    });
});
