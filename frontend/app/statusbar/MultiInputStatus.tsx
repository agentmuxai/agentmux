// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { shortcutFor } from "@/app/keybindings";
import { isTermMultiInput, setIsTermMultiInput } from "@/store/global";
import { Show, type JSX } from "solid-js";

/**
 * "Type into all terminals" is on: what you type in the focused terminal pane
 * goes to every shell terminal pane in the window. One mode for the whole
 * window, so it shows here once rather than in every terminal's header.
 * Click to turn it off.
 */
const MultiInputStatus = (): JSX.Element => {
    const tip = () => {
        const keys = shortcutFor("term:multiInput");
        return `Typing into all terminals. Click to turn off${keys ? ` (${keys})` : ""}`;
    };
    return (
        <Show when={isTermMultiInput()}>
            <div
                class="status-bar-item clickable"
                onClick={() => setIsTermMultiInput(false)}
                data-tip={tip()}
                aria-label={tip()}
                role="button"
            >
                <i class="fa-solid fa-keyboard status-icon" style={{ color: "var(--warning-color)" }} />
                <span style={{ color: "var(--warning-color)" }}>Multi Input</span>
            </div>
        </Show>
    );
};

MultiInputStatus.displayName = "MultiInputStatus";

export { MultiInputStatus };
