// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * BashOutputViewer - Displays bash command and output with exit code
 */

import clsx from "clsx";
import React, { memo } from "react";
import type { BashParams, BashResult } from "../types";

interface BashOutputViewerProps {
    params: BashParams;
    result?: BashResult;
}

export const BashOutputViewer: React.FC<BashOutputViewerProps> = memo(({ params, result }) => {
    const hasOutput = result && (result.stdout || result.stderr);
    const hasError = result && result.exitCode !== 0;

    return (
        <div className="agent-bash">
            <div className="agent-bash-cmd">
                <span className="agent-bash-dollar">$</span> {params.command}
            </div>
            {hasOutput && (
                <pre className={clsx("agent-bash-output", { "has-error": hasError })}>
                    {result.stdout}
                    {result.stderr && (
                        <span className="agent-bash-stderr">{result.stderr}</span>
                    )}
                </pre>
            )}
            {result && (
                <div
                    className={clsx("agent-bash-exit", {
                        "exit-success": result.exitCode === 0,
                        "exit-error": result.exitCode !== 0,
                    })}
                >
                    Exit code: {result.exitCode}
                </div>
            )}
        </div>
    );
});

BashOutputViewer.displayName = "BashOutputViewer";
