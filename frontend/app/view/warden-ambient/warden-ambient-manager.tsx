// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Warden — Ambient section: how AgentMux's own model calls (session titles, names, prompt
// suggestions, narration, running summaries, Supervisor verdicts) have gone since its server
// started, one row per purpose. These calls write the line under each agent in the Swarm and the
// pane tab's tooltip, so this is where that line's health is watched. It was the Swarm's Stats panel.
// docs/reports/REPORT_SWARM_SECTIONS_AND_AMBIENT_IN_WARDEN_2026_10_21.md §6.

import { createEffect, createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { summarizeAmbientOutcomes, type AmbientOutcomes } from "@/app/store/ambient-outcomes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { pollWhileVisible } from "@/app/view/warden-shared/warden-shared";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./warden-ambient-manager.scss";

export const WardenAmbientManager = (props: {
    visible?: boolean;
    /** How many purposes are failing, for the pane's title. */
    onFailingCount?: (n: number) => void;
}): JSX.Element => {
    const [outcomes, setOutcomes] = createSignal<AmbientOutcomes | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(true);

    const refresh = async () => {
        try {
            setOutcomes((await RpcApi.AmbientOutcomesCommand(TabRpcClient, {})) ?? null);
            setError(null);
        } catch (e) {
            // The last counts stay: a failed refresh says nothing about them.
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    // Polls only while the section is shown (warden-view.tsx passes `visible`).
    pollWhileVisible(() => void refresh(), () => props.visible ?? true);

    const summary = createMemo(() => summarizeAmbientOutcomes(outcomes()));
    createEffect(() => props.onFailingCount?.(summary()?.unhealthyCount ?? 0));

    return (
        <div class="warden-manager-body">
            <p class="warden-manager-summary">
                AgentMux's own model calls since its server started: session titles, names, prompt suggestions,
                narration and more
            </p>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show
                when={summary()}
                fallback={
                    // With an error and no counts yet, the error alone: the server couldn't be asked.
                    <Show when={loading() || !error()}>
                        <div class="warden-section-stub">
                            {loading() ? "Loading…" : "No calls have ended since AgentMux's server started."}
                        </div>
                    </Show>
                }
            >
                {(s) => (
                    <>
                        <table class="warden-manager-table warden-ambient-table">
                            <thead>
                                <tr>
                                    <th>Purpose</th>
                                    <th>Requests</th>
                                    <th>Accepted</th>
                                    <th>Kept</th>
                                    <th>Refused</th>
                                    <th>Skipped</th>
                                    <th>Failed</th>
                                </tr>
                            </thead>
                            <tbody>
                                <For each={s().rows}>
                                    {(row) => (
                                        <tr data-state={row.unhealthy ? "failing" : undefined} title={row.detail}>
                                            <td class="warden-ambient-purpose">
                                                {row.name}
                                                <Show when={row.unhealthy}>
                                                    <span class="warden-ambient-failing">failing</span>
                                                </Show>
                                            </td>
                                            <td class="warden-ambient-num">{row.total}</td>
                                            <td class="warden-ambient-num">{row.counts.accepted}</td>
                                            <td class="warden-ambient-num">{row.counts.kept}</td>
                                            <td class="warden-ambient-num">{row.counts.refused}</td>
                                            <td class="warden-ambient-num">{row.counts.skipped}</td>
                                            <td class="warden-ambient-num">{row.counts.failed}</td>
                                        </tr>
                                    )}
                                </For>
                            </tbody>
                            <tfoot>
                                <tr>
                                    <td>All</td>
                                    <td class="warden-ambient-num">{s().total}</td>
                                    <td colSpan={5} />
                                </tr>
                            </tfoot>
                        </table>
                        <p class="warden-manager-footnote">
                            Requests include those that never reached the model. Accepted: an answer that passed the
                            checks. Kept: nothing needed to change. A purpose is failing when its refusals and failures
                            outnumber both. Counts start again when AgentMux's server restarts.
                        </p>
                    </>
                )}
            </Show>
        </div>
    );
};
