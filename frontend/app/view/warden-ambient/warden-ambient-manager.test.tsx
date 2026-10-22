// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const outcomes = vi.hoisted(() => vi.fn());

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AmbientOutcomesCommand: () => outcomes() } }));

import { WardenAmbientManager } from "./warden-ambient-manager";

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

const rowOf = (name: string) => screen.getByText(name).closest("tr")!;
const cells = (tr: Element) => Array.from(tr.querySelectorAll("td")).map((td) => td.textContent);

describe("WardenAmbientManager", () => {
    it("shows one row per purpose with its counts by bucket, and the total", async () => {
        outcomes.mockResolvedValue({
            activity_summary: { accepted: 10, kept: 30 },
            next_prompt_suggestion: { accepted: 5, timeout: 1 },
        });
        render(() => <WardenAmbientManager visible />);
        await waitFor(() => expect(screen.getByText("Session titles")).toBeInTheDocument());
        expect(cells(rowOf("Session titles"))).toEqual(["Session titles", "40", "10", "30", "0", "0", "0"]);
        expect(cells(rowOf("Prompt suggestions"))).toEqual(["Prompt suggestions", "6", "5", "0", "0", "0", "1"]);
        expect(cells(rowOf("All"))[1]).toBe("46");
    });

    it("marks a failing purpose and reports how many there are", async () => {
        outcomes.mockResolvedValue({ activity_summary_pushed: { accepted: 1, cli_failed: 4 }, activity_summary: { kept: 3 } });
        const onFailingCount = vi.fn();
        render(() => <WardenAmbientManager visible onFailingCount={onFailingCount} />);
        await waitFor(() => expect(onFailingCount).toHaveBeenLastCalledWith(1));
        expect(rowOf("Session titles (recovery)").dataset.state).toBe("failing");
        expect(rowOf("Session titles").dataset.state).toBeUndefined();
    });

    it("says so when no call has ended yet", async () => {
        outcomes.mockResolvedValue({});
        render(() => <WardenAmbientManager visible />);
        await waitFor(() => expect(screen.getByText(/No calls have ended/)).toBeInTheDocument());
        expect(document.querySelector("table")).toBeNull();
    });

    it("shows only the error from a server without the command, not that no calls have ended", async () => {
        outcomes.mockRejectedValue(new Error("unknown command ambient.outcomes"));
        render(() => <WardenAmbientManager visible />);
        await waitFor(() => expect(screen.getByText(/unknown command ambient\.outcomes/)).toBeInTheDocument());
        expect(screen.queryByText(/No calls have ended/)).toBeNull();
        expect(screen.queryByText("Loading…")).toBeNull();
    });

    it("doesn't ask while hidden", async () => {
        outcomes.mockResolvedValue({ activity_summary: { accepted: 1 } });
        render(() => <WardenAmbientManager visible={false} />);
        await Promise.resolve();
        expect(outcomes).not.toHaveBeenCalled();
    });
});
