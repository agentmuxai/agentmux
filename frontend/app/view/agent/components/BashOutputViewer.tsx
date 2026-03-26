// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BashOutputViewer - Displays bash command and output with exit code
 */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";
import type { BashParams, BashResult } from "../types";

interface BashOutputViewerProps {
    params: BashParams;
    result?: BashResult;
}

export const BashOutputViewer = ({ params, result }: BashOutputViewerProps): JSX.Element => {
    const hasOutput = result && (result.stdout || result.stderr);
    const hasError = result && result.exitCode !== 0;

    return (
        <div class="agent-bash">
            <div class="agent-bash-cmd">
                <span class="agent-bash-dollar">$</span> {params.command}
            </div>
            <Show when={hasOutput}>
                <pre class={clsx("agent-bash-output", { "has-error": hasError })}>
                    {result.stdout}
                    <Show when={result.stderr}>
                        <span class="agent-bash-stderr">{result.stderr}</span>
                    </Show>
                </pre>
            </Show>
            <Show when={result}>
                <div
                    class={clsx("agent-bash-exit", {
                        "exit-success": result.exitCode === 0,
                        "exit-error": result.exitCode !== 0,
                    })}
                >
                    Exit code: {result.exitCode}
                </div>
            </Show>
        </div>
    );
};

BashOutputViewer.displayName = "BashOutputViewer";
