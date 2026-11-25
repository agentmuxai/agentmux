// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// AgentLaunchCost — at the top of an agent's Personal Memory: what its next
// launch delivers into its context, part by part, in tokens, and what that
// takes of its context window when the window is known
// (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.4).

import { For, Show, type JSX } from "solid-js";
import type { LaunchCost } from "@/types/rpc/LaunchCost";
import {
    bandWords,
    deliveredParts,
    deliveredTotal,
    launchBudget,
    launchGroups,
    viaLabel,
    type AgentWindow,
} from "./agent-launch-cost";
import { topContributors, topContributorsLabel } from "./memory-aggregates";
import { shareLabel, tokensLabel, tokensTitle, TOKEN_ESTIMATE_NOTE } from "./memory-tokens";
import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";

export function AgentLaunchCost(props: { cost: LaunchCost; window: AgentWindow | null }): JSX.Element {
    const total = () => deliveredTotal(props.cost);
    const budget = () => (props.window ? launchBudget(total(), props.window.window) : null);
    const top = () => topContributors(deliveredParts(props.cost).map((p) => p.tokens_est));
    return (
        <section class="agent-launch-cost" aria-label={t("launchCost.label")} data-testid="agent-launch-cost">
            <div class="agent-launch-cost-head">
                <span {...tip(tokensTitle(total()))}>
                    <Rich k="launchCost.startsWith" params={{ tokens: tokensLabel(total()) }} />
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
                    {t("launchCost.ownFile", { file: props.cost.startup_file ?? "" })}
                </p>
            </Show>
            <div class="agent-launch-cost-bar" aria-hidden="true">
                <For each={launchGroups(props.cost)}>
                    {(g) => (
                        <span
                            class={`agent-launch-cost-segment agent-launch-cost-segment--${g.key}`}
                            style={{ "flex-grow": g.tokens }}
                            {...tip(`${g.label} ${tokensLabel(g.tokens)}`, { label: true })}
                        />
                    )}
                </For>
            </div>
            <table class="agent-launch-cost-table">
                <thead>
                    <tr>
                        <th scope="col">{t("launchCost.part")}</th>
                        <th scope="col">{t("launchCost.arrivesBy")}</th>
                        <th scope="col" {...tip(TOKEN_ESTIMATE_NOTE)}>
                            {t("launchCost.tokens")}
                        </th>
                        <th scope="col">{t("launchCost.share")}</th>
                    </tr>
                </thead>
                <tbody>
                    <For each={deliveredParts(props.cost)}>
                        {(p) => (
                            <tr>
                                <td>{p.name}</td>
                                <td>{viaLabel(p)}</td>
                                <td {...tip(tokensTitle(p.tokens_est, p.size_bytes || undefined, p.lines))}>
                                    {tokensLabel(p.tokens_est)}
                                </td>
                                <td>{shareLabel(p.tokens_est, total())}</td>
                            </tr>
                        )}
                    </For>
                </tbody>
            </table>
            <Show when={top()}>
                {(lead) => <p class="agent-launch-cost-note">{topContributorsLabel(lead(), "parts", "the launch")}.</p>}
            </Show>
            <Show when={budget()}>
                {(b) => (
                    <p class="agent-launch-cost-note">
                        {t("launchCost.room", { tokens: tokensLabel(b().room), model: props.window?.model ?? t("launchCost.itsModel") })}
                    </p>
                )}
            </Show>
        </section>
    );
}
