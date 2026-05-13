// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BashOutputViewer - Displays bash command and output with exit code
 */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";
import type { BashParams, BashResult } from "../types";
import { HighlightedCode } from "./HighlightedCode";

interface BashOutputViewerProps {
    params: BashParams;
    result?: BashResult;
}

export const BashOutputViewer = ({ params, result }: BashOutputViewerProps): JSX.Element => {
    // Tool result may come back as either the structured BashResult
    // shape (Claude Code's tool_use_result: stdout/stderr/exitCode)
    // or as the loose `{ content: "<string>" }` fallback when the
    // translator can't find a structured field. Treat both — the
    // user just wants to see the output.
    const looseResult = result as Record<string, unknown> | undefined;
    const stdout =
        (looseResult?.stdout as string | undefined) ??
        (looseResult?.content as string | undefined) ??
        "";
    const stderr = (looseResult?.stderr as string | undefined) ?? "";
    const exitCode = looseResult?.exitCode as number | undefined;

    const hasOutput = stdout.length > 0 || stderr.length > 0;
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
                <pre class={clsx("agent-bash-output", { "has-error": hasError })}>
                    {stdout}
                    <Show when={stderr}>
                        <span class="agent-bash-stderr">{stderr}</span>
                    </Show>
                </pre>
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
