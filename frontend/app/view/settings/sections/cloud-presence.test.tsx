// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings › Cloud presence: the line from `presence.status`, and Publish now. */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { PresenceStatusResult } from "@/app/store/rpc-api";

const status = vi.fn();
const publishNow = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        PresenceStatusCommand: (...args: unknown[]) => status(...args),
        PresencePublishNowCommand: (...args: unknown[]) => publishNow(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { CloudPresence, POLL_MS, presenceLine, span } from "./cloud-presence";

const NOW = 1_791_000_000_000;

function at(state: PresenceStatusResult["state"], extra: Partial<PresenceStatusResult> = {}): PresenceStatusResult {
    return { state, since_ms: NOW - 60_000, record_version: 2, ...extra };
}

describe("span", () => {
    it("reads seconds, minutes and hours", () => {
        expect(span(12_000)).toBe("12 s");
        expect(span(4 * 60_000)).toBe("4 min");
        expect(span(2 * 3_600_000)).toBe("2 h");
        expect(span(-5_000)).toBe("0 s");
    });
});

describe("presenceLine", () => {
    it("says what each state means, in plain words", () => {
        expect(presenceLine(at("publishing", { last_ok_ms: NOW - 12_000 }), NOW)).toBe("Published 12 s ago");
        expect(presenceLine(at("signed_out"), NOW)).toBe("Sign in to publish");
        expect(
            presenceLine(at("retrying", { next_try_ms: NOW + 40_000, last_error: "cloud unreachable" }), NOW)
        ).toBe("Retrying in 40 s (cloud unreachable)");
        expect(presenceLine(at("unsupported", { next_try_ms: NOW + 4 * 60_000 }), NOW)).toBe(
            "The cloud doesn't accept presence yet. Checking again in 4 min"
        );
        expect(
            presenceLine(at("rejected", { next_try_ms: NOW + 9 * 60_000, last_error: "revoked" }), NOW)
        ).toBe("The cloud refused this computer's record (revoked). Trying again in 9 min");
    });

    it("says why an install doesn't publish, and when it said goodbye", () => {
        expect(presenceLine(at("off", { off_reason: "dev_build" }), NOW)).toBe("Off for dev builds");
        expect(presenceLine(at("off", { off_reason: "setting" }), NOW)).toBe("Off (setting)");
        expect(presenceLine(at("off", { off_reason: "headless" }), NOW)).toBe("Off for headless installs");
        expect(presenceLine(at("off", { off_reason: "isolated_home" }), NOW)).toBe("Off for isolated homes");
        expect(presenceLine(at("off", { off_reason: "test_harness" }), NOW)).toBe("Off under test");
        expect(presenceLine(at("off"), NOW)).toBe("Off");
        expect(presenceLine(at("signed_off"), NOW)).toBe("Signed off: your devices show this computer as offline");
    });

    it("says now once a try is due", () => {
        expect(presenceLine(at("retrying", { next_try_ms: NOW - 1_000 }), NOW)).toBe("Retrying now");
    });
});

describe("CloudPresence", () => {
    beforeEach(() => {
        status.mockReset();
        publishNow.mockReset();
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("shows the line and the note", async () => {
        status.mockResolvedValue(
            at("publishing", {
                last_ok_ms: Date.now() - 12_000,
                offset_ms: 720_000,
                note: "Your clock differs from the cloud's by 12 min",
            })
        );
        render(() => <CloudPresence />);
        expect(await screen.findByRole("status")).toHaveTextContent(/^Published 1\d s ago$/);
        expect(screen.getByText("Your clock differs from the cloud's by 12 min")).toBeInTheDocument();
    });

    it("Publish now asks for a try and reads the status again", async () => {
        status.mockResolvedValueOnce(at("unsupported", { next_try_ms: Date.now() + 240_000 }));
        status.mockResolvedValue(at("publishing", { last_ok_ms: Date.now() }));
        publishNow.mockResolvedValue({ started: true });
        render(() => <CloudPresence />);
        await screen.findByText(/doesn't accept presence yet/);
        fireEvent.click(screen.getByRole("button", { name: "Publish now" }));
        await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(/^Published \d s ago$/));
        expect(publishNow).toHaveBeenCalledTimes(1);
    });

    it("can't publish while signed out", async () => {
        status.mockResolvedValue(at("signed_out"));
        render(() => <CloudPresence />);
        await screen.findByText("Sign in to publish");
        expect(screen.getByRole("button", { name: "Publish now" })).toBeDisabled();
    });

    it.each([
        ["off", { off_reason: "dev_build" as const }, "Off for dev builds"],
        ["signed_off", {}, "Signed off: your devices show this computer as offline"],
    ] as const)("can't publish while %s", async (state, extra, line) => {
        status.mockResolvedValue(at(state, extra));
        render(() => <CloudPresence />);
        await screen.findByText(line);
        expect(screen.getByRole("button", { name: "Publish now" })).toBeDisabled();
    });

    it("says why there is no status", async () => {
        status.mockRejectedValue(new Error("Cloud presence hasn't started on this install."));
        render(() => <CloudPresence />);
        expect(await screen.findByRole("status")).toHaveTextContent("Cloud presence hasn't started on this install.");
        expect(screen.getByRole("button", { name: "Publish now" })).toBeDisabled();
    });

    it("reads the status again while it is shown, and stops when it isn't", async () => {
        vi.useFakeTimers();
        status.mockResolvedValue(at("retrying", { next_try_ms: Date.now() + 40_000, last_error: "cloud unreachable" }));
        const { unmount } = render(() => <CloudPresence />);
        await vi.advanceTimersByTimeAsync(0);
        expect(status).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(POLL_MS);
        expect(status).toHaveBeenCalledTimes(2);
        expect(screen.getByRole("status")).toHaveTextContent("Retrying in 37 s (cloud unreachable)");
        unmount();
        await vi.advanceTimersByTimeAsync(POLL_MS * 3);
        expect(status).toHaveBeenCalledTimes(2);
    });
});
