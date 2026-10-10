// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { TowerProcess } from "@/app/store/rpc-api";
import { type JSX, Show } from "solid-js";

/** A process row's name. AgentMux's own processes say what they are ("GPU",
 *  "Renderer · window Main"), since many run the same executable; the
 *  executable's name follows, muted. */
export function ProcessName(props: { process: TowerProcess }): JSX.Element {
    return (
        <Show
            when={props.process.detail}
            fallback={<span class="tower-label">{props.process.name || `PID ${props.process.pid}`}</span>}
        >
            {(detail) => (
                <>
                    <span class="tower-label">{detail()}</span>
                    <span class="tower-muted tower-exe">{props.process.name}</span>
                </>
            )}
        </Show>
    );
}
