// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * OutputHiddenMarker — the single, consistent affordance shown wherever a
 * tool-output body is render-capped (SPEC_TOOL_OUTPUT_CAP_2026_05_30.md §6).
 * A cap is never silent. Phase 2 makes this click into the full-output
 * pane; for now it is informational (the full output is preserved in state).
 */

import { type JSX } from "solid-js";

interface OutputHiddenMarkerProps {
    /** How many units were dropped. */
    hidden: number;
    /** Unit noun — "line" for text bodies, "block" for the chunk log,
     *  "result" for result-card lists (search results, …), "row" for tables. */
    noun: "line" | "block" | "result" | "row";
    /** "tail" hides older content (marker sits above the body); "head"
     *  hides later content (marker sits below). Default "tail". */
    from?: "head" | "tail";
}

export function OutputHiddenMarker(props: OutputHiddenMarkerProps): JSX.Element {
    const direction = (): string => (props.from === "head" ? "more" : "earlier");
    const plural = (): string => (props.hidden === 1 ? "" : "s");
    return (
        <div
            class="agent-output-hidden-marker"
            title="Rendering capped — the full output is preserved"
        >
            … {props.hidden.toLocaleString()} {direction()} {props.noun}
            {plural()} hidden
        </div>
    );
}

OutputHiddenMarker.displayName = "OutputHiddenMarker";
