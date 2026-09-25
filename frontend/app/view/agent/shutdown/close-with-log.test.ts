// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const closePane = vi.fn();
vi.mock("@/store/services", () => ({ ObjectService: { ClosePane: (...a: unknown[]) => closePane(...a) } }));
vi.mock("@/app/store/flash-notifications", () => ({ pushFlashError: vi.fn() }));
const removeMovedBlock = vi.fn();
vi.mock("@/layout/lib/layoutMagnify", () => ({ removeMovedBlock: (...a: unknown[]) => removeMovedBlock(...a) }));
vi.mock("@/layout/lib/layoutModelHooks", () => ({ getLayoutModelForTabById: () => ({}) }));
const log = { begin: vi.fn(), end: vi.fn(), fail: vi.fn() };
vi.mock("./shutdown-log", () => ({
    beginShutdownLog: (id: string) => log.begin(id),
    endShutdownLog: (id: string) => log.end(id),
    failShutdownLog: (id: string, m: string) => log.fail(id, m),
}));

import { closeWithShutdownLog } from "./close-with-log";

describe("closeWithShutdownLog (SPEC_AGENT_SELF_QUIT §5.5)", () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it("asks srv to close and wait, with a log for each agent member", async () => {
        closePane.mockResolvedValue({});
        await closeWithShutdownLog("t1", ["a", "term"], (id) => id === "a");
        expect(closePane).toHaveBeenCalledWith(["a", "term"], true);
        expect(log.begin.mock.calls).toEqual([["a"]]);
        expect(log.fail).not.toHaveBeenCalled();
    });

    it("a rejection before the close started shows in the log, so the pane never hangs on 'Shutting down…'", async () => {
        closePane.mockRejectedValue(new Error("ClosePane: tab not found: t1"));
        await closeWithShutdownLog("t1", ["a"], () => true);
        expect(log.fail).toHaveBeenCalledWith("a", expect.stringContaining("tab not found"));
    });

    it("when every block is already gone it drops them from the layout and ends their logs", async () => {
        closePane.mockRejectedValue(new Error("ClosePane: block not found: a"));
        await closeWithShutdownLog("t1", ["a"], () => true);
        expect(removeMovedBlock).toHaveBeenCalledWith({}, "a");
        expect(log.end).toHaveBeenCalledWith("a");
        expect(log.fail).not.toHaveBeenCalled();
    });
});
