// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { TowerProcess } from "@/app/store/rpc-api";
import { type JSX, Show } from "solid-js";

/** A process row's name. AgentMux's own processes say what they are ("GPU",
 *  "Renderer · window Main"), since many run the same executable; the
 *  executable's name follows, muted. The process an agent's tool call runs
 *  says which call ("⟵ Run the srv tests"), as the agent described it
 *  (SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md §3.1, §5.3). */
export function ProcessName(props: { process: TowerProcess }): JSX.Element {
    return (
        <>
            <Show
                when={props.process.detail}
                fallback={<span class="tower-label">{props.process.name || `PID ${props.process.pid}`}</span>}
            >
                {(detail) => (
                    <>
                        <span class="tower-label">{detail()}</span>
                        <span class="tower-muted">{props.process.name}</span>
                    </>
                )}
            </Show>
            <Show when={props.process.started_by}>
                {(call) => (
                    <span class="tower-started-by" title={`Started for the agent's tool call: ${call()}`}>
                        ⟵ {call()}
                    </span>
                )}
            </Show>
        </>
    );
}
