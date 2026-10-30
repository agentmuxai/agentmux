// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SystemNoteBlock: one of AgentMux's own notes (`[AgentMux] ...`) in the agent
 * pane, a quiet container that is never the human's turn
 * (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md). P2 folds it into the
 * one origin container as its `system` variant.
 */

import { Show, type JSX } from "solid-js";
import type { SystemNoteNode } from "../types";

export function SystemNoteBlock(props: { node: SystemNoteNode }): JSX.Element {
    const cli = () => props.node.source === "cli";
    return (
        <div class="agent-system-note" classList={{ "agent-system-note--cli": cli() }}>
            <span class="agent-system-note__label">
                <i class={cli() ? "fa-solid fa-terminal" : "fa-solid fa-circle-info"} aria-hidden="true" /> {cli() ? "CLI message" : "AgentMux"}
            </span>
            <Show when={props.node.delay}>
                {(delay) => <span class="agent-system-note__delay">{delay()}</span>}
            </Show>
            {/* Text only: the note is never read as markup. */}
            <span class="agent-system-note__text">{props.node.text}</span>
        </div>
    );
}
