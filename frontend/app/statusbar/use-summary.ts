// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { attentionItems, sharedAttentionRows, type AttentionItem } from "@/app/notification/attention-list";
import { waitingApps, type WaitingApp } from "@/app/notification/waiting-for-you";
import { runtimeCounts } from "@/app/store/agent-runtime";
import { useKanbanNeeds } from "@/app/view/kanban/kanban-held";
import { vaultRequests, vaultUses } from "@/app/view/vault/vault-store";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import { createMemo, type Accessor } from "solid-js";
import { summarize, type Summary } from "./summary";

export interface SummaryStatusProps {
    /** Agent rows, for how many have a process; default is the live ones. */
    rows?: Accessor<AgentRuntime[]>;
    items?: Accessor<AttentionItem[]>;
    apps?: Accessor<WaitingApp[]>;
    uses?: Accessor<UseRequest[]>;
    requests?: Accessor<VaultRequest[]>;
    /** How many Kanban tasks need the person (the Kanban pane's own count); default is the live one. */
    tasks?: Accessor<number>;
}

/** The summary of what waits on the person, from what this window already knows. */
export function useSummary(props: SummaryStatusProps): Accessor<Summary> {
    const tasks = props.tasks ?? useKanbanNeeds();
    return createMemo(() =>
        summarize({
            running: runtimeCounts((props.rows ?? sharedAttentionRows)()).running,
            items: (props.items ?? attentionItems)(),
            apps: (props.apps ?? waitingApps)(),
            uses: (props.uses ?? vaultUses)(),
            requests: (props.requests ?? vaultRequests)(),
            tasks: tasks(),
        })
    );
}
