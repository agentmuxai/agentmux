// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BashOutputViewer - Displays bash command and output with exit code
 */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";
import type { BashParams, BashResult } from "../types";
import { HighlightedCode } from "./HighlightedCode";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { capText, MAX_TOOL_OUTPUT_LINES } from "./output-cap";
import { parseExitPrefix } from "../tool-meta/bash-exit";

interface BashOutputViewerProps {
    params: BashParams;
    result?: BashResult;
}

// The `<exited N in Ts>` prefix bashwrap injects, and its parser, live in
// tool-meta/bash-exit.ts (shared with the row's exit pill).

export const BashOutputViewer = ({ params, result }: BashOutputViewerProps): JSX.Element => {
    // Tool result may come back as either the structured BashResult
    // shape (Claude Code's tool_use_result: stdout/stderr/interrupted)
    // or as the loose `{ content: "<string>" }` fallback when the
    // translator can't find a structured field. Treat both — the
    // user just wants to see the output.
    const looseResult = result as unknown as Record<string, unknown> | undefined;
    const rawStdout =
        (looseResult?.stdout as string | undefined) ??
        (looseResult?.content as string | undefined) ??
        "";
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

    const hasOutput = stdoutCap.text.length > 0 || stderrCap.text.length > 0;
    const hasError = exitCode !== undefined && exitCode !== 0;

    return (
        <div class="agent-bash">
            <div class="agent-bash-cmd">
                <span class="agent-bash-dollar">$</span>
                <HighlightedCode
                    code={params.command}
                    lang="bash"
                    class="agent-bash-cmd-code"
                />
            </div>
            <Show when={hasOutput}>
                <Show when={stdoutCap.hiddenLines > 0}>
                    <OutputHiddenMarker hidden={stdoutCap.hiddenLines} noun="line" from="tail" />
                </Show>
                <Show when={stdoutCap.text}>
                    <pre class={clsx("agent-bash-output", { "has-error": hasError })}>
                        {stdoutCap.text}
                    </pre>
                </Show>
                <Show when={stderrCap.text}>
                    <Show when={stderrCap.hiddenLines > 0}>
                        <OutputHiddenMarker hidden={stderrCap.hiddenLines} noun="line" from="tail" />
                    </Show>
                    <pre class={clsx("agent-bash-output agent-bash-stderr", { "has-error": hasError })}>
                        {stderrCap.text}
                    </pre>
                </Show>
            </Show>
            <Show when={exitCode !== undefined}>
                <div
                    class={clsx("agent-bash-exit", {
                        "exit-success": exitCode === 0,
                        "exit-error": exitCode !== 0,
                    })}
                >
                    Exit code: {exitCode}
                </div>
            </Show>
        </div>
    );
};

BashOutputViewer.displayName = "BashOutputViewer";
