// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The file-drop indicator a pane draws while files are dragged over the
 * window (SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §4):
 *   armed   — a faint dashed outline: this pane can take the drop;
 *   target  — a translucent accent tint, an accent border and a prompt
 *             saying what will happen;
 *   blocked — no tint, the prompt in the warning style with the reason.
 * Rendered by PaneChrome over the whole pane; never takes pointer events.
 */

import { Show } from "solid-js";
import type { PaneDropState } from "./file-drop";
import "./drop-indicators.scss";

export function DropIndicator(props: { state: PaneDropState }) {
    const text = () => {
        const s = props.state;
        return s.state === "target" ? s.message : s.state === "blocked" ? s.reason : undefined;
    };
    const icon = () => {
        const s = props.state;
        return s.state === "blocked" ? "fa-ban" : s.state === "target" ? (s.icon ?? "fa-file-arrow-down") : "";
    };
    return (
        <div class={`drop-indicator drop-indicator--${props.state.state}`} aria-hidden="true">
            <Show when={text()}>
                <span
                    class="drop-indicator__prompt"
                    classList={{ "drop-indicator__prompt--blocked": props.state.state === "blocked" }}
                >
                    <i class={`fa-solid ${icon()}`} />
                    {text()}
                </span>
            </Show>
        </div>
    );
}
