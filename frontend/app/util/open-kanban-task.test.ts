// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A task link opened from anywhere is one step of the window's trail (pane-history.ts). */

import { afterEach, describe, expect, it, vi } from "vitest";

const opened = vi.hoisted(() => vi.fn(async () => {}));
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: opened }));

import { installTrailRunners } from "@/app/store/trail-hooks";
import { openKanbanTask } from "./open-kanban-task";

let restore: (() => void) | undefined;
afterEach(() => restore?.());

describe("openKanbanTask", () => {
    it("opens the Kanban on the task, as a gesture of the trail", async () => {
        const log: string[] = [];
        restore = installTrailRunners(
            async (run) => {
                log.push("gesture:start");
                const r = await run();
                log.push("gesture:end");
                return r;
            },
            (run) => Promise.resolve(run())
        );
        opened.mockImplementationOnce(async () => void log.push("opened"));
        await openKanbanTask("12");
        expect(log).toEqual(["gesture:start", "opened", "gesture:end"]);
        expect(opened).toHaveBeenCalledWith("kanban", { meta: { view: "kanban", "kanban:open": "12" } }, { "kanban:open": "12" });
    });
});
