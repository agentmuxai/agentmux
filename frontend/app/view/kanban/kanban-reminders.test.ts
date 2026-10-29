// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ setConfig: [] as Record<string, unknown>[] }));

vi.mock("@/store/rpc-api", () => ({
    RpcApi: {
        SetConfigCommand: async (_client: unknown, data: Record<string, unknown>) => {
            h.setConfig.push(data);
        },
    },
}));
vi.mock("@/store/rpc-util", () => ({ TabRpcClient: {} }));

import { KANBAN_REMINDERS_SETTING, kanbanRemindersOn, setKanbanReminders } from "./kanban-reminders";

describe("the Kanban's backlog reminder setting", () => {
    beforeEach(() => {
        h.setConfig.length = 0;
    });

    it("is off unless turned on, as srv reads it", () => {
        expect(kanbanRemindersOn(undefined)).toBe(false);
        expect(kanbanRemindersOn({})).toBe(false);
        expect(kanbanRemindersOn({ [KANBAN_REMINDERS_SETTING]: false })).toBe(false);
        expect(kanbanRemindersOn({ [KANBAN_REMINDERS_SETTING]: "true" })).toBe(false);
        expect(kanbanRemindersOn({ [KANBAN_REMINDERS_SETTING]: true })).toBe(true);
    });

    it("writes the one key both switches read", async () => {
        setKanbanReminders(true);
        setKanbanReminders(false);
        await vi.waitFor(() => expect(h.setConfig).toHaveLength(2));
        expect(h.setConfig).toEqual([{ "kanban:reminders": true }, { "kanban:reminders": false }]);
    });
});
