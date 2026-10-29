// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Preparing…" with the phase, above the composer, while a container agent's container is started
 * in the background (`preparing.ts`). Not a working turn: no spinner of the Working row, no timer,
 * and it does not queue what the user types.
 */
import { Show, createMemo, type Accessor, type JSX } from "solid-js";
import { AsciiSpinner } from "@/app/element/AsciiSpinner";
import { preparingNow } from "../preparing";
import type { DocumentNode } from "../types";

export function AgentPreparingRow(props: { nodes: Accessor<readonly DocumentNode[]>; enabled: Accessor<boolean>; now?: () => number }): JSX.Element {
    const preparing = createMemo(() => (props.enabled() ? preparingNow(props.nodes(), (props.now ?? Date.now)()) : null));
    return (
        <Show when={preparing()}>
            {(p) => (
                <div class="agent-preparing-row" role="status" data-testid="agent-preparing-row">
                    <AsciiSpinner class="agent-preparing-spinner" />
                    <span class="agent-preparing-title">Preparing…</span>
                    <span class="agent-preparing-phase">{p().phase}</span>
                </div>
            )}
        </Show>
    );
}
AgentPreparingRow.displayName = "AgentPreparingRow";
