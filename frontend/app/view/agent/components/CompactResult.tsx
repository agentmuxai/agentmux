// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CompactResult - Shows tool results as a compact summary with expand-to-JSON.
 *
 * Default view: a short one-line description extracted from the result shape.
 * Expanded view: pretty-printed JSON, render-capped per
 * SPEC_TOOL_OUTPUT_CAP_2026_05_30.md so a large structured result can't bloat
 * the conversation DOM once expanded.
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

export const CompactResult = ({ tool, params, result }: CompactResultProps): JSX.Element => {
    const [expanded, setExpanded] = createSignal(tool === "Glob");

    const summary = summarize(tool, params, result);
    // When the result carries a terminal-style string body (stdout/output/
    // content), render it as a terminal instead of a JSON blob. Structured
    // results (no string body) fall back to JSON.
    const termText = terminalText(result);
    const fullJson = result != null ? JSON.stringify(result, null, 2) : "";
    // Expandable when there's more to show than the one-line summary. A terminal
    // body is worth expanding whenever it's multi-line (the summary collapses
    // it) or longer than the summary.
    const hasDetail = termText != null
        ? termText.includes("\n") || termText.length > summary.length
        : fullJson.length > summary.length + 10;
    // Head-cap the expanded JSON so a large structured payload (Glob / Grep /
    // Agent) can't add an unbounded <pre> once the summary is expanded.
    const jsonCap = capText(fullJson, MAX_TOOL_OUTPUT_LINES, "head");

    return (
        <div class="agent-tool-compact-result">
            <div
                class="agent-tool-compact-summary"
                classList={{ clickable: hasDetail }}
                onClick={() => hasDetail && setExpanded(!expanded())}
                title={hasDetail ? (expanded() ? "Collapse" : "Expand full result") : undefined}
            >
                <Show when={hasDetail}>
                    <span class="agent-tool-compact-chevron">{expanded() ? "▾" : "▸"}</span>
                </Show>
                <span class="agent-tool-compact-text">{summary}</span>
            </div>
            <Show when={expanded()}>
                {tool === "Glob" && Array.isArray(result?.files)
                    ? (() => {
                        const files: string[] = result.files;
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
                    : termText != null
                    ? <TerminalOutput text={termText} from="tail" />
                    : (
                        <>
                            <pre class="agent-tool-compact-json">{jsonCap.text}</pre>
                            <Show when={jsonCap.hiddenLines > 0}>
                                <OutputHiddenMarker hidden={jsonCap.hiddenLines} noun="line" from="head" />
                            </Show>
                        </>
                    )
                }
            </Show>
        </div>
    );
};
