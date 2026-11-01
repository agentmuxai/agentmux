// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SetupNote: AgentMux's own setup prompts, the start-up document and the opening update (both begin
 * "# Session Context"), as one muted line in the transcript that opens on a click. They are for the
 * agent, not the person, so they are never the human's turn and never a coloured card: this is the
 * origin container's quiet style (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md §4).
 * Open or closed is the pin (`documentState.pinnedNodes`), like the row it replaces.
 */

import { LinkifiedText } from "@/app/element/linkified-text";
import { Button } from "@/app/element/ui/Button";
import { Show, type JSX } from "solid-js";
import type { UserMessageNode } from "../types";
import { previewBox } from "./scroll-handoff";

/** The capped text box skids and hands scroll to the pane at its edges, like every preview box. */
const handoff = previewBox();

const OPENING_UPDATE_RE = /^# Session Context\s+## Opening update\b/;

/** What the line says: the opening update is a question to the agent, anything else is the session's setup. */
export function setupLabel(message: string): string {
    return OPENING_UPDATE_RE.test(message) ? "Asked for an opening update" : "Session set up";
}

interface SetupNoteProps {
    node: UserMessageNode;
    /** Open: the whole text under the line. */
    pinned: boolean;
    onTogglePin: () => void;
}

export function SetupNote(props: SetupNoteProps): JSX.Element {
    const label = () => setupLabel(props.node.message);
    return (
        <div
            class="agent-origin agent-origin--setup"
            classList={{ "agent-origin--open": props.pinned }}
            data-origin="setup"
            data-testid="agent-setup-note"
        >
            <Button
                variant="row"
                density="compact"
                class="agent-origin__setup-toggle"
                onClick={props.onTogglePin}
                aria-expanded={props.pinned}
                aria-label={`${label()}: ${props.pinned ? "click to close" : "click to read"}`}
            >
                <i
                    class={`fa-solid fa-chevron-${props.pinned ? "down" : "right"} agent-origin__setup-chevron`}
                    aria-hidden="true"
                />
                <span class="agent-origin__label">{label()}</span>
            </Button>
            <Show when={props.pinned}>
                {/* Text only: a prompt is shown as written, never read as markup. */}
                <pre class="agent-origin__setup-text" ref={handoff}>
                    <LinkifiedText text={props.node.message} />
                </pre>
            </Show>
        </div>
    );
}

SetupNote.displayName = "SetupNote";
