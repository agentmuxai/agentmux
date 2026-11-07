// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// LogView, the views: the spacer as tall as the whole log and, inside it, the window of rows that is drawn, each row
// split into its ANSI colour runs. (log-view-state.ts says which rows.)

import { For, type JSX } from "solid-js";

import { parseAnsi } from "./ansi";
import { LOG_ROW_PX, type LogViewState } from "./log-view-state";

export function LogRows(props: { s: LogViewState }): JSX.Element {
    const s = props.s;
    return (
        <div class="install-log-spacer" style={{ height: `${s.total() * LOG_ROW_PX}px` }}>
            <div class="install-log-window" style={{ transform: `translateY(${s.visible().first * LOG_ROW_PX}px)` }}>
                <For each={s.visible().rows}>
                    {(row) => (
                        <div class="install-log-line" data-tone={row.tone}>
                            <For each={parseAnsi(row.text)}>
                                {(seg) => (seg.cls ? <span class={seg.cls}>{seg.text}</span> : seg.text)}
                            </For>
                        </div>
                    )}
                </For>
            </div>
        </div>
    );
}
