// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm's Stats panel, opened from the toolbar's Stats button: how the model
 * calls AgentMux makes on its own (session titles, names, prompt suggestions,
 * narration) have gone since its server started, one row per purpose. It slides
 * down from the top, under the toolbar, and pushes the list down.
 * docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md section 5.8.
 */

import { createMemo, For, Show, type JSX } from "solid-js";
import { summarizeAmbientOutcomes } from "@/app/store/ambient-outcomes";
import type { SwarmViewModel } from "./swarm-model";
import "./swarm-stats.scss";

export function SwarmStatsPanel(props: { model: SwarmViewModel }): JSX.Element {
    const summary = createMemo(() => summarizeAmbientOutcomes(props.model.ambientOutcomesAtom()));
    return (
        <Show when={summary()}>
            {(s) => (
                // Always mounted once there are counts, so opening and closing
                // can animate; `inert` keeps the closed panel out of tab order.
                <div class="swarm-stats" data-open={props.model.statsOpenAtom() ? "" : undefined}>
                    <div class="swarm-stats-inner" inert={!props.model.statsOpenAtom()}>
                        <div class="swarm-stats-title">
                            AgentMux's own model calls since its server started · {s().total}
                        </div>
                        {/* One grid for all rows so the counts line up; each row is
                            `display: contents`, so the hover sits on both cells. */}
                        <div class="swarm-stats-rows">
                            <For each={s().rows}>
                                {(row) => (
                                    <div class="swarm-stats-row">
                                        <span class="swarm-stats-name" title={row.detail}>
                                            {row.name}
                                        </span>
                                        <span
                                            classList={{ "swarm-stats-counts": true, "swarm-stats-counts--warn": row.unhealthy }}
                                            title={row.detail}
                                        >
                                            {row.text}
                                        </span>
                                    </div>
                                )}
                            </For>
                        </div>
                    </div>
                </div>
            )}
        </Show>
    );
}
