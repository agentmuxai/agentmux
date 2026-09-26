// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CompactResult - the default tool result body.
 *
 * A text body (stdout / output / content) is shown as-is: one line as plain
 * text, several as a terminal. A structured result shows a short one-line
 * description with `▸` expand-to-JSON, render-capped per
 * SPEC_TOOL_OUTPUT_CAP_2026_05_30.md so a large payload can't bloat the
 * conversation DOM once expanded.
 */

import { For, createSignal, Show, type JSX } from "solid-js";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { capText, MAX_TOOL_OUTPUT_LINES } from "./output-cap";
import { TerminalOutput } from "./TerminalOutput";
import { terminalText } from "./terminal-text";

interface CompactResultProps {
    tool: string;
    params: Record<string, any>;
    result: any;
}

/**
 * Extract a compact, human-readable summary from a tool result.
 */
function summarize(tool: string, params: Record<string, any>, result: any): string {
    if (result == null) return "No output";
    if (typeof result === "string") {
        return result.length > 120 ? result.slice(0, 120) + "..." : result;
    }

    // Tool-specific compact summaries
    switch (tool) {
        case "Grep": {
            // { matches: [...] } or { content: "..." } or raw string output
            if (Array.isArray(result.matches)) {
                const n = result.matches.length;
                return `${n} match${n === 1 ? "" : "es"} found`;
            }
            if (result.content && typeof result.content === "string") {
                const lines = result.content.split("\n").filter((l: string) => l.trim());
                return `${lines.length} result line${lines.length === 1 ? "" : "s"}`;
            }
            break;
        }
        case "Glob": {
            if (Array.isArray(result.files)) {
                const n = result.files.length;
                const preview = result.files.slice(0, 3).map(shortPath).join(", ");
                return n <= 3 ? preview : `${preview} (+${n - 3} more)`;
            }
            break;
        }
        case "Agent": {
            if (result.content && typeof result.content === "string") {
                const trimmed = result.content.trim();
                return trimmed.length > 150 ? trimmed.slice(0, 150) + "..." : trimmed;
            }
            break;
        }
        case "Task": {
            if (result.status) return `Status: ${result.status}`;
            break;
        }
        case "Workflow": {
            if (result.status) return `Status: ${result.status}`;
            if (result.content && typeof result.content === "string") {
                const trimmed = result.content.trim();
                return trimmed.length > 150 ? trimmed.slice(0, 150) + "..." : trimmed;
            }
            break;
        }
    }

    // Generic: extract known content fields
    if (result.content && typeof result.content === "string") {
        const trimmed = result.content.trim();
        return trimmed.length > 120 ? trimmed.slice(0, 120) + "..." : trimmed;
    }
    if (result.output && typeof result.output === "string") {
        const trimmed = result.output.trim();
        return trimmed.length > 120 ? trimmed.slice(0, 120) + "..." : trimmed;
    }

    // Fallback: count keys
    const keys = Object.keys(result);
    if (keys.length === 0) return "Empty result";
    if (keys.length <= 3) {
        return keys.map((k) => `${k}: ${compactValue(result[k])}`).join(", ");
    }
    return `{${keys.slice(0, 3).join(", ")} +${keys.length - 3} more}`;
}

function compactValue(val: any): string {
    if (val == null) return "null";
    if (typeof val === "string") return val.length > 40 ? `"${val.slice(0, 40)}..."` : `"${val}"`;
    if (typeof val === "number" || typeof val === "boolean") return String(val);
    if (Array.isArray(val)) return `[${val.length} items]`;
    if (typeof val === "object") return `{${Object.keys(val).length} keys}`;
    return String(val);
}

function shortPath(p: string): string {
    const parts = p.replace(/\\/g, "/").split("/");
    return parts.length <= 2 ? p : ".../" + parts.slice(-2).join("/");
}

export const CompactResult = (props: CompactResultProps): JSX.Element => {
    // `props.x`, never destructured: a destructured prop is frozen at mount,
    // so a caller that kept this mounted across a result update would show
    // stale output (see ToolBlock.tsx's reactivity note).
    //
    // A terminal-style string body (stdout/output/content) is shown directly:
    // the tool panel around it is already the expand/collapse control, and a
    // second `▸` inside it was the "tree parent" that hid the content
    // (SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.5). One line reads as
    // plain text; several lines render as a terminal. Only a structured result
    // (no string body) keeps the one-line summary + `▸` JSON.
    const termText = () => terminalText(props.result);
    return (
        <Show
            when={termText() == null}
            fallback={
                <div class="agent-tool-compact-result">
                    <Show
                        when={termText()!.trim().includes("\n")}
                        fallback={<div class="agent-tool-compact-line">{termText()!.trim()}</div>}
                    >
                        <TerminalOutput text={termText()!} from="tail" />
                    </Show>
                </div>
            }
        >
            <StructuredResult tool={props.tool} params={props.params} result={props.result} />
        </Show>
    );
};

function StructuredResult(props: CompactResultProps): JSX.Element {
    const [expanded, setExpanded] = createSignal(props.tool === "Glob");

    const summary = () => summarize(props.tool, props.params, props.result);
    const fullJson = () => (props.result != null ? JSON.stringify(props.result, null, 2) : "");
    // Expandable when there's more to show than the one-line summary.
    const hasDetail = () => fullJson().length > summary().length + 10;
    // Head-cap the expanded JSON so a large structured payload (Glob / Grep /
    // Agent) can't add an unbounded <pre> once the summary is expanded.
    const jsonCap = () => capText(fullJson(), MAX_TOOL_OUTPUT_LINES, "head");

    return (
        <div class="agent-tool-compact-result">
            <div
                class="agent-tool-compact-summary"
                classList={{ clickable: hasDetail() }}
                onClick={() => hasDetail() && setExpanded(!expanded())}
                title={hasDetail() ? (expanded() ? "Collapse" : "Expand full result") : undefined}
            >
                <Show when={hasDetail()}>
                    <span class="agent-tool-compact-chevron">{expanded() ? "▾" : "▸"}</span>
                </Show>
                <span class="agent-tool-compact-text">{summary()}</span>
            </div>
            <Show when={expanded()}>
                {props.tool === "Glob" && Array.isArray(props.result?.files)
                    ? (() => {
                        const files: string[] = props.result.files;
                        const visible = files.slice(0, MAX_TOOL_OUTPUT_LINES);
                        const hidden = files.length - visible.length;
                        return (
                            <>
                                <div class="agent-tool-glob-files">
                                    <For each={visible}>
                                        {(f: string) => <div class="agent-tool-glob-file">{f}</div>}
                                    </For>
                                </div>
                                <Show when={hidden > 0}>
                                    <OutputHiddenMarker hidden={hidden} noun="line" from="head" />
                                </Show>
                            </>
                        );
                    })()
                    : (
                        <>
                            <pre class="agent-tool-compact-json">{jsonCap().text}</pre>
                            <Show when={jsonCap().hiddenLines > 0}>
                                <OutputHiddenMarker hidden={jsonCap().hiddenLines} noun="line" from="head" />
                            </Show>
                        </>
                    )
                }
            </Show>
        </div>
    );
}
