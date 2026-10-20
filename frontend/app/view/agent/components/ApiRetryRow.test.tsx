// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toApiRetryView, type ApiRetryView } from "../hooks/useApiRetryState";
import { ApiRetryRow, apiRetryText } from "./ApiRetryRow";

const NOW = 1_800_000_000_000;

const retry = (over: Partial<ApiRetryView> = {}): ApiRetryView => ({
    attempt: 3,
    maxRetries: 10,
    nextRetryAt: NOW + 18_000,
    errorStatus: null,
    error: "unknown",
    ...over,
});

describe("toApiRetryView", () => {
    it("puts the next attempt on this clock, from the time remaining srv sent", () => {
        const sent = { attempt: 3, maxRetries: 10, retryInMs: 18_000, errorStatus: null, error: "unknown" };
        expect(toApiRetryView(sent, NOW)).toEqual({
            attempt: 3,
            maxRetries: 10,
            nextRetryAt: NOW + 18_000,
            errorStatus: null,
            error: "unknown",
        });
        expect(toApiRetryView(null, NOW)).toBeNull();
    });
});

describe("apiRetryText", () => {
    it("names a connection failure, the attempt and the wait", () => {
        expect(apiRetryText(retry(), NOW)).toBe("Can't reach the API. Retrying: attempt 3 of 10, next in 18 s.");
    });

    it("rounds the wait up and says when the attempt is due", () => {
        expect(apiRetryText(retry(), NOW + 17_100)).toContain("next in 1 s.");
        expect(apiRetryText(retry(), NOW + 18_000)).toBe("Can't reach the API. Retrying now: attempt 3 of 10.");
    });

    it("names an HTTP status when one came back", () => {
        expect(apiRetryText(retry({ errorStatus: 529 }), NOW)).toMatch(/^The API is overloaded\./);
        expect(apiRetryText(retry({ errorStatus: 429 }), NOW)).toMatch(/^Rate-limited by the API\./);
        expect(apiRetryText(retry({ errorStatus: 500 }), NOW)).toMatch(/\(HTTP 500\)\./);
    });
});

describe("ApiRetryRow", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        vi.setSystemTime(NOW);
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("shows the attempt and counts down to the next one", () => {
        render(() => <ApiRetryRow retry={retry()} onStop={() => {}} />);
        expect(screen.getByRole("status").textContent).toContain("attempt 3 of 10, next in 18 s.");
        vi.advanceTimersByTime(5_000);
        expect(screen.getByRole("status").textContent).toContain("next in 13 s.");
    });

    it("Stop calls the stop action", () => {
        const onStop = vi.fn();
        render(() => <ApiRetryRow retry={retry()} onStop={onStop} />);
        fireEvent.click(screen.getByRole("button", { name: /Stop/ }));
        expect(onStop).toHaveBeenCalledTimes(1);
    });

    it("follows a new attempt and disappears when cleared", () => {
        const [state, setState] = createSignal<ApiRetryView | null>(retry());
        render(() => <ApiRetryRow retry={state()} onStop={() => {}} />);
        setState(retry({ attempt: 4, nextRetryAt: NOW + 30_000 }));
        expect(screen.getByRole("status").textContent).toContain("attempt 4 of 10, next in 30 s.");
        setState(null);
        expect(screen.queryByRole("status")).toBeNull();
        expect(screen.queryByRole("button", { name: /Stop/ })).toBeNull();
    });
});
