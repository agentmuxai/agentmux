// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import english from "@/app/i18n/locales/en.json";
import { KANBAN_REMINDERS_DESCRIPTION } from "@/app/view/kanban/kanban-reminders";

describe("the Kanban reminders setting", () => {
    // The Kanban toolbar's tooltip still reads the constant; the Settings row reads the dictionary. One sentence, two homes:
    // this keeps them from drifting apart until the Kanban reads the dictionary too.
    it("says in Settings what the Kanban toolbar says", () => {
        expect((english as Record<string, string>)["settings.advanced.kanbanReminders.description"]).toBe(KANBAN_REMINDERS_DESCRIPTION);
    });
});
