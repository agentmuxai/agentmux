// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BashOutputViewer — a finished Bash call's output and exit code. The command
 * itself is in the tool row and its hover, so it isn't printed again here
 * (REPORT_THEME_MENU_RUNTIME_PANEL_TOOL_PREVIEW_TWEAKS_2026_10_07.md §3).
 */

import clsx from "clsx";
import { Show, createMemo, type JSX } from "solid-js";
import type { BashResult } from "../types";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { capText, MAX_TOOL_OUTPUT_LINES } from "./output-cap";
import { parseExitPrefix } from "../tool-meta/bash-exit";

interface BashOutputViewerProps {
    result?: BashResult;
}

// The `<exited N in Ts>` prefix bashwrap injects, and its parser, live in
// tool-meta/bash-exit.ts (shared with the row's exit pill).

export const BashOutputViewer = (props: BashOutputViewerProps): JSX.Element => {
    // `props.x` and accessors, never destructured: a destructured prop is
    // frozen at mount, so a viewer kept mounted across a result update would
    // keep showing the first result (SPEC_AGENT_PANE_PREVIEW_CLEANUPS §4).
    //
    // Tool result may come back as either the structured BashResult
    // shape (Claude Code's tool_use_result: stdout/stderr/interrupted)
    // or as the loose `{ content: "<string>" }` fallback when the
    // translator can't find a structured field. Treat both — the
    // user just wants to see the output.
    const view = createMemo(() => {
        const looseResult = props.result as unknown as Record<string, unknown> | undefined;
        const rawStdout =
            (looseResult?.stdout as string | undefined) ?? (looseResult?.content as string | undefined) ?? "";
        const stderr = (looseResult?.stderr as string | undefined) ?? "";
        const nativeExit = looseResult?.exitCode as number | undefined;
        // Recover the exit code from the `<exited N in Ts>` prefix that
        // bashwrap injects, when the result lacks a native exitCode field.
        // Strip the prefix from stdout regardless — the user shouldn't
        // see the marker.
        const { exit: parsedExit, body: rawBody } = parseExitPrefix(rawStdout);
        const exitCode = nativeExit ?? parsedExit;
        // Cap each body to bound the conversation DOM (SPEC_TOOL_OUTPUT_CAP).
        // Tail-keep — the latest output is what matters for a command.
        const stdoutCap = capText(rawBody, MAX_TOOL_OUTPUT_LINES, "tail");
        const stderrCap = capText(stderr, MAX_TOOL_OUTPUT_LINES, "tail");
        return {
            exitCode,
            stdoutCap,
            stderrCap,
            hasOutput: stdoutCap.text.length > 0 || stderrCap.text.length > 0,
            hasError: exitCode !== undefined && exitCode !== 0,
        };
    });

    return (
        <div class="agent-bash">
            <Show when={view().hasOutput} fallback={<div class="agent-bash-no-output">No output</div>}>
                <Show when={view().stdoutCap.hiddenLines > 0}>
                    <OutputHiddenMarker hidden={view().stdoutCap.hiddenLines} noun="line" from="tail" />
                </Show>
                <Show when={view().stdoutCap.text}>
                    <pre class={clsx("agent-bash-output", { "has-error": view().hasError })}>
                        {view().stdoutCap.text}
                    </pre>
                </Show>
                <Show when={view().stderrCap.text}>
                    <Show when={view().stderrCap.hiddenLines > 0}>
                        <OutputHiddenMarker hidden={view().stderrCap.hiddenLines} noun="line" from="tail" />
                    </Show>
                    <pre class={clsx("agent-bash-output agent-bash-stderr", { "has-error": view().hasError })}>
                        {view().stderrCap.text}
                    </pre>
                </Show>
            </Show>
            <Show when={view().exitCode !== undefined}>
                <div
                    class={clsx("agent-bash-exit", {
                        "exit-success": view().exitCode === 0,
                        "exit-error": view().exitCode !== 0,
                    })}
                >
                    Exit code: {view().exitCode}
                </div>
            </Show>
        </div>
    );
};

BashOutputViewer.displayName = "BashOutputViewer";
