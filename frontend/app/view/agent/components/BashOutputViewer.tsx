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

// `agentmux-bashwrap` prepends every captured bash run with a single
// line like `<exited 0 in 0.60s>` (or `<exited 1 in 1.23s>` on
// failure). It's the only durable carrier of the exit code through
// Claude's tool_use_result, which doesn't include an `exitCode`
// field. We strip the prefix when rendering stdout and use it as
// the fallback exit-code source when the result didn't carry one
// natively.
const EXIT_PREFIX_RE = /^<exited (-?\d+) in [\d.]+s>\n?/;

function parseExitPrefix(s: string | undefined): {
    exit: number | undefined;
    body: string;
} {
    if (!s) return { exit: undefined, body: "" };
    const m = s.match(EXIT_PREFIX_RE);
    if (!m) return { exit: undefined, body: s };
    return { exit: parseInt(m[1], 10), body: s.slice(m[0].length) };
}

export const BashOutputViewer = ({ params, result }: BashOutputViewerProps): JSX.Element => {
    // Tool result may come back as either the structured BashResult
    // shape (Claude Code's tool_use_result: stdout/stderr/interrupted)
    // or as the loose `{ content: "<string>" }` fallback when the
    // translator can't find a structured field. Treat both — the
    // user just wants to see the output.
    const looseResult = result as Record<string, unknown> | undefined;
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
    const { exit: parsedExit, body: stdout } = parseExitPrefix(rawStdout);
    const exitCode = nativeExit ?? parsedExit;

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
