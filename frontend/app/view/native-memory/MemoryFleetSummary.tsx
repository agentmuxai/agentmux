// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// MemoryFleetSummary — above Memory → Personal's agent grid: every agent's
// Personal Memory together, the median per agent, and the agents with more
// than twice that median (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.5).

import { For, Show, type JSX } from "solid-js";
import { fleetSummary, fleetSummaryLabel, type FleetAgent } from "./memory-aggregates";
import { tokensLabel, tokensTitle, TOKEN_ESTIMATE_NOTE } from "./memory-tokens";

export function MemoryFleetSummary(props: { agents: FleetAgent[] }): JSX.Element {
    const fleet = () => fleetSummary(props.agents);
    return (
        <Show when={fleet()}>
            {(f) => (
                <div class="memory-fleet-summary" data-testid="memory-fleet-summary" title={TOKEN_ESTIMATE_NOTE}>
                    <span>Personal Memory: {fleetSummaryLabel(f())}</span>
                    <Show when={f().outliers.length > 0}>
                        <span class="memory-fleet-summary-outliers">
                            {" · "}over twice the median:{" "}
                            <For each={f().outliers}>
                                {(a, i) => (
                                    <>
                                        {i() > 0 ? ", " : ""}
                                        <span title={tokensTitle(a.tokens)}>
                                            {a.name} {tokensLabel(a.tokens)}
                                        </span>
                                    </>
                                )}
                            </For>
                        </span>
                    </Show>
                </div>
            )}
        </Show>
    );
}
