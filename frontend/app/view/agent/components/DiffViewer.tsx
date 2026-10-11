// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DiffViewer — an Edit's change as a diff, highlighted per line.
 *
 * The diff is built by the preview text stage (`preview-text/docs.ts`): from
 * the tool's own unified diff when it returned one, otherwise from the edit's
 * old and new strings, with tabs expanded and one shared dedent across both
 * sides. `PreviewLines` draws it with the `+`/`-` marker in its own box and the
 * line's background across the whole line, and highlights each side with its
 * own token stream (docs/reports/REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md
 * §5.1, §5.2).
 */

import { createMemo, Show, type JSX } from "solid-js";
import { diffDocFromSides, diffDocFromUnified } from "../preview-text/docs";
import type { PreviewDoc } from "../preview-text/types";
import type { EditParams, EditResult } from "../types";
import { PreviewLines } from "./PreviewLines";

interface DiffViewerProps {
    params: EditParams;
    result?: EditResult;
    /** ToolNode.status — gates the params fallback. Only synthesise a diff
     *  when the edit actually applied. For failed/denied edits the params
     *  reflect the *intended* change, not what happened, so showing them
     *  as a diff would mislead. Defaults to "success" when omitted. */
    status?: string;
}

export const DiffViewer = (props: DiffViewerProps): JSX.Element => {
    const filePath = () => props.params.file_path ?? "";
    const doc = createMemo((): PreviewDoc | null => {
        // Prefer the pre-computed diff from result (populated by some providers).
        if (props.result?.diff) return diffDocFromUnified(props.result.diff, filePath());
        // Fall back to computing from params only for successful edits.
        // For failed/denied nodes params reflect the *intended* change —
        // synthesising a diff would show what was supposed to happen, not
        // what did, misleading the user. (Codex P2 on PR #1561.)
        if ((props.status ?? "success") !== "success") return null;
        const p = props.params;
        if (!p || (p.old_string == null && p.new_string == null)) return null;
        return diffDocFromSides(p.old_string ?? "", p.new_string ?? "", filePath());
    });

    return (
        <Show
            when={doc()}
            fallback={
                <pre class="agent-diff-empty">
                    No diff available
                    {"\n"}
                    File: {filePath()}
                </pre>
            }
        >
            <div class="agent-diff">
                <div class="agent-diff-header">{filePath()}</div>
                <PreviewLines doc={doc()!} class="agent-diff-body" />
            </div>
        </Show>
    );
};

DiffViewer.displayName = "DiffViewer";
