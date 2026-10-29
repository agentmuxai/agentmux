// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const outcomesCmd = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AmbientOutcomesCommand: outcomesCmd } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { ambientFailingCount, refreshAmbient, watchAmbient } from "@/app/store/ambient-health";
import { AmbientPurposes } from "./AmbientPurposes";

const healthy = { activity_summary: { accepted: 10, kept: 30 } };
const failing = { activity_summary: { accepted: 1, "rejected:shape": 2, cli_failed: 2 } };

describe("AmbientPurposes", () => {
    beforeEach(() => {
        outcomesCmd.mockReset();
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("lists a line per purpose and opens one to its counts", async () => {
        outcomesCmd.mockResolvedValue(healthy);
        render(() => <AmbientPurposes />);
        const head = await screen.findByRole("button", { name: /Session titles/ });
        expect(screen.queryByText("Accepted")).toBeNull();
        fireEvent.click(head);
        expect(screen.getByText("Accepted").nextElementSibling?.textContent).toBe("10");
        expect(screen.getByText("Kept").nextElementSibling?.textContent).toBe("30");
        expect(screen.getByText("Requests").nextElementSibling?.textContent).toBe("40");
    });

    it("marks the failing count on the line without opening it", async () => {
        outcomesCmd.mockResolvedValue(failing);
        render(() => <AmbientPurposes />);
        expect(await screen.findByText("2 failed")).toBeTruthy();
    });

    it("says so when the counts could not be read, and keeps the last ones", async () => {
        outcomesCmd.mockResolvedValueOnce(healthy);
        render(() => <AmbientPurposes />);
        await screen.findByRole("button", { name: /Session titles/ });
        outcomesCmd.mockImplementation(async () => {
            throw new Error("down");
        });
        await refreshAmbient();
        expect(await screen.findByText(/Could not read the counts/)).toBeTruthy();
        expect(screen.getByRole("button", { name: /Session titles/ })).toBeTruthy();
    });

    it("keeps a purpose open when the next ask brings new row objects", async () => {
        outcomesCmd.mockResolvedValue(healthy);
        render(() => <AmbientPurposes />);
        fireEvent.click(await screen.findByRole("button", { name: /Session titles/ }));
        expect(screen.getByText("Accepted")).toBeTruthy();
        outcomesCmd.mockResolvedValue({ activity_summary: { accepted: 11, kept: 30 } });
        await refreshAmbient();
        await waitFor(() => expect(screen.getByText("Accepted").nextElementSibling?.textContent).toBe("11"));
        expect(screen.getByRole("button", { name: /Session titles/ }).getAttribute("aria-expanded")).toBe("true");
    });
});

describe("watchAmbient", () => {
    beforeEach(() => {
        outcomesCmd.mockReset();
    });
    afterEach(() => vi.useRealTimers());

    it("asks now and then on the period until stopped; a failing purpose is counted", async () => {
        vi.useFakeTimers();
        outcomesCmd.mockResolvedValue(failing);
        const stop = watchAmbient(5000);
        expect(outcomesCmd).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(10_000);
        expect(outcomesCmd).toHaveBeenCalledTimes(3);
        expect(ambientFailingCount()).toBe(1);
        stop();
        await vi.advanceTimersByTimeAsync(10_000);
        expect(outcomesCmd).toHaveBeenCalledTimes(3);
    });
});
