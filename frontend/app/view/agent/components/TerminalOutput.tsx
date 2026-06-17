// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TerminalOutput — render a captured terminal/tool output body as a monospace,
 * ANSI-colored, scroll-capped terminal panel instead of a JSON blob.
 *
 * Reuses `AnsiLine` (SGR coloring) and the shared output cap
 * (`MAX_TOOL_OUTPUT_LINES`) + `OutputHiddenMarker` so a large body can't bloat
 * the conversation DOM — matching `CompactResult` / `BashOutputViewer`. A thin
 * wrapper over `AnsiLine` is deliberately preferred to embedding xterm.js in the
 * feed (the term pane's full VT emulator is heavy and stateful for what static,
 * colored scrollback needs).
 *
 * Lives next to the other tool-output renderers (not in `element/`, where the
 * spec sketched it) so the view → element dependency direction stays correct:
 * it imports the view-layer cap utilities, and `AnsiLine` lives in `element/`.
 * See docs/specs/SPEC_TOOL_OUTPUT_TEE_AND_TERMINAL_RENDER_2026_06_17.md §4.
 */

import clsx from "clsx";
import { For, Show, createMemo, type JSX } from "solid-js";
import AnsiLine from "@/element/ansiline";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { capText, MAX_TOOL_OUTPUT_LINES } from "./output-cap";

interface TerminalOutputProps {
    text: string;
    /** Extra class on the container. */
    class?: string;
    /** Cap direction — command/log output keeps the tail (latest matters);
     *  read-top-down content keeps the head. Default "tail". */
    from?: "head" | "tail";
}

export function TerminalOutput(props: TerminalOutputProps): JSX.Element {
    const from = (): "head" | "tail" => props.from ?? "tail";
    const capped = createMemo(() => capText(props.text ?? "", MAX_TOOL_OUTPUT_LINES, from()));
    const lines = createMemo(() => capped().text.split("\n"));

    return (
        <div class={clsx("agent-terminal-output", props.class)}>
            {/* tail-cap hides OLDER lines → marker above the body */}
            <Show when={from() === "tail" && capped().hiddenLines > 0}>
                <OutputHiddenMarker hidden={capped().hiddenLines} noun="line" from="tail" />
            </Show>
            <For each={lines()}>{(line) => <AnsiLine line={line} />}</For>
            {/* head-cap hides LATER lines → marker below the body */}
            <Show when={from() === "head" && capped().hiddenLines > 0}>
                <OutputHiddenMarker hidden={capped().hiddenLines} noun="line" from="head" />
            </Show>
        </div>
    );
}

TerminalOutput.displayName = "TerminalOutput";
