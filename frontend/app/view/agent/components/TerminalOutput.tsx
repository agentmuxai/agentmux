// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TerminalOutput — a captured terminal or tool output body (Grep, Glob, MCP
 * text, Task, the default renderer), as the terminal showed it.
 *
 * The text goes through the preview text stage (`preview-text/docs.ts`
 * `outputDoc`): colour codes kept as spans across lines, carriage-return
 * redraws applied, other escape codes dropped, tabs at 8, capped at the end
 * that matters. Long lines scroll sideways instead of wrapping mid-word, the
 * same as Bash output and code
 * (docs/reports/REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §3.3, §5.3).
 * See docs/specs/SPEC_TOOL_OUTPUT_TEE_AND_TERMINAL_RENDER_2026_06_17.md §4.
 */

import clsx from "clsx";
import { createMemo, type JSX } from "solid-js";
import { outputDoc } from "../preview-text/docs";
import { PreviewLines } from "./PreviewLines";

interface TerminalOutputProps {
    text: string;
    /** Extra class on the container. */
    class?: string;
    /** Cap direction — command/log output keeps the tail (latest matters);
     *  read-top-down content keeps the head. Default "tail". */
    from?: "head" | "tail";
}

export function TerminalOutput(props: TerminalOutputProps): JSX.Element {
    const doc = createMemo(() => outputDoc(props.text ?? "", { from: props.from ?? "tail" }));
    return <PreviewLines doc={doc()} class={clsx("agent-terminal-output", props.class)} />;
}

TerminalOutput.displayName = "TerminalOutput";
