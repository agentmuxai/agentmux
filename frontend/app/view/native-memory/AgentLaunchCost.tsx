// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// AgentLaunchCost — at the top of an agent's Personal Memory: what its next
// launch delivers into its context, part by part, in tokens, and what that
// takes of its context window when the window is known
// (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.4).

import { For, Show, type JSX } from "solid-js";
import type { LaunchCost } from "@/types/rpc/LaunchCost";
import { bandWords, launchBudget, launchGroups, viaLabel, type AgentWindow } from "./agent-launch-cost";
import { shareLabel, tokensLabel, tokensTitle, TOKEN_ESTIMATE_NOTE } from "./memory-tokens";

export function AgentLaunchCost(props: { cost: LaunchCost; window: AgentWindow | null }): JSX.Element {
    const budget = () => (props.window ? launchBudget(props.cost.total_tokens_est, props.window.window) : null);
    return (
        <section class="agent-launch-cost" aria-label="What this agent starts with" data-testid="agent-launch-cost">
            <div class="agent-launch-cost-head">
                <span title={tokensTitle(props.cost.total_tokens_est)}>
                    Starts with <strong>{tokensLabel(props.cost.total_tokens_est)}</strong>
                </span>
                <Show when={budget()}>
                    {(b) => (
                        <span classList={{ "agent-launch-cost-band": true, [`agent-launch-cost-band--${b().band}`]: true }}>
                            {b().share}
                            {bandWords(b().band) ? ` · ${bandWords(b().band)}` : ""}
                        </span>
                    )}
                </Show>
            </div>
            <Show when={props.cost.startup_file && !props.cost.delivered_by_file}>
                <p class="agent-launch-cost-note">
                    {props.cost.startup_file} in this agent's folder is your own file, so its launch doesn't add
                    AgentMux's startup text to it.
                </p>
            </Show>
            <div class="agent-launch-cost-bar" aria-hidden="true">
                <For each={launchGroups(props.cost)}>
                    {(g) => (
                        <span
                            class={`agent-launch-cost-segment agent-launch-cost-segment--${g.key}`}
                            style={{ "flex-grow": g.tokens }}
                            title={`${g.label} ${tokensLabel(g.tokens)}`}
                        />
                    )}
                </For>
            </div>
            <table class="agent-launch-cost-table">
                <thead>
                    <tr>
                        <th scope="col">Part</th>
                        <th scope="col">Arrives by</th>
                        <th scope="col" title={TOKEN_ESTIMATE_NOTE}>
                            Tokens
                        </th>
                        <th scope="col">Share</th>
                    </tr>
                </thead>
                <tbody>
                    <For each={props.cost.parts}>
                        {(p) => (
                            <tr>
                                <td>{p.name}</td>
                                <td>{viaLabel(p)}</td>
                                <td title={tokensTitle(p.tokens_est, p.size_bytes || undefined, p.lines)}>
                                    {tokensLabel(p.tokens_est)}
                                </td>
                                <td>{shareLabel(p.tokens_est, props.cost.total_tokens_est)}</td>
                            </tr>
                        )}
                    </For>
                </tbody>
            </table>
            <Show when={budget()}>
                {(b) => (
                    <p class="agent-launch-cost-note">
                        {tokensLabel(b().room)} left before auto-compaction, at launch, on{" "}
                        {props.window?.model ?? "its model"}.
                    </p>
                )}
            </Show>
        </section>
    );
}
