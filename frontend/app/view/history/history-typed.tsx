// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The History pane, Typed: what the user typed to an agent, newest
// first, from the history srv keeps with the agent (the composer's ArrowUp and
// Ctrl+R read the same; docs/specs/SPEC_AGENT_INPUT_HISTORY_2026_10_18.md).
// Forgetting an entry, or an agent's whole history, is asked twice.

import { Show, type JSX } from "solid-js";

import { createHistoryTyped } from "./history-typed-state";
import { TypedBar, TypedEntries, TypedFootnote, TypedTrash } from "./history-typed-parts";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./history.scss";

export const HistoryTyped = (props: { visible: boolean }): JSX.Element => {
    const t = createHistoryTyped(props);
    return (
        <div class="warden-history-typed">
            <TypedBar t={t} />
            <Show when={t.error()}>
                <div class="warden-manager-error">⚠ {t.error()}</div>
            </Show>
            <TypedEntries t={t} />
            <TypedTrash t={t} />
            <TypedFootnote t={t} />
        </div>
    );
};

HistoryTyped.displayName = "HistoryTyped";
