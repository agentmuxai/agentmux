// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, on, untrack, type Accessor, type Setter } from "solid-js";

import type { AuditFilter } from "./audit-shared";

/** The pane's `audit:agent` changing (the Swarm's "Audit for this agent") sets the filter, which starts the list over. */
export function followAgentProp(
    agent: Accessor<string | undefined>,
    filter: Accessor<AuditFilter>,
    setFilter: Setter<AuditFilter>,
    setAgentText: Setter<string>
): void {
    createEffect(
        on(
            agent,
            (name) => {
                if (name === undefined) return;
                setAgentText(name);
                setFilter({ ...untrack(filter), agent: name });
            },
            { defer: true }
        )
    );
}
