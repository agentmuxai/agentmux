// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Kanban's backlog reminder setting, shared by its two switches (Settings ›
 * Advanced and the Kanban's toolbar) so they read and describe it the same way.
 * srv reads it in `crates/srv/src/backend/kanban.rs` (`reminders_enabled`).
 * SPEC_KANBAN_REMINDER_OFF_BY_DEFAULT_2026_10_22.md.
 */

import { RpcApi } from "@/store/rpc-api";
import { TabRpcClient } from "@/store/rpc-util";
import { fireAndForget } from "@/util/util";

export const KANBAN_REMINDERS_SETTING = "kanban:reminders";

export const KANBAN_REMINDERS_DESCRIPTION =
    "An agent with tasks waiting in its Kanban backlog gets a note after 2 minutes idle, once each time it goes idle";

/** Whether reminders are on: only when turned on, as srv reads it. */
export function kanbanRemindersOn(settings: Record<string, unknown> | null | undefined): boolean {
    return settings?.[KANBAN_REMINDERS_SETTING] === true;
}

/** Turn reminders on or off for this install; srv picks it up at its next look (30 s). */
export function setKanbanReminders(on: boolean): void {
    fireAndForget(() => RpcApi.SetConfigCommand(TabRpcClient, { [KANBAN_REMINDERS_SETTING]: on } as any));
}
