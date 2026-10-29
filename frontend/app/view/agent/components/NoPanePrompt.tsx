// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NoPanePrompt: what My Agents asks when the user clicks an agent that runs with no pane
 * (always-available, parked). "Open it" attaches it to this pane, "Fork…" makes a named copy.
 * It sits in the row's fork popover, next to the already-open prompt's buttons; the popover
 * owns the close button (top right) and the click-outside and Escape dismissal for both prompts.
 */

import { Button } from "@/app/element/ui";
import type { JSX } from "solid-js";

interface NoPanePromptProps {
    /** The agent's name, as its row shows it. */
    name: string;
    onOpen: () => void;
    onFork: () => void;
}

export const NoPanePrompt = (props: NoPanePromptProps): JSX.Element => (
    <>
        <span class="agent-fork-prompt-msg">
            <strong>{props.name}</strong> is running with no pane.
        </span>
        <div class="agent-fork-prompt-actions">
            <Button
                tone="accent"
                class="agent-fork-btn agent-fork-btn--primary"
                onClick={props.onOpen}
                data-testid="agent-fork-attach"
            >
                Open it
            </Button>
            <Button class="agent-fork-btn agent-fork-btn--secondary" onClick={props.onFork} data-testid="agent-fork-fork">
                Fork…
            </Button>
        </div>
    </>
);

NoPanePrompt.displayName = "NoPanePrompt";
