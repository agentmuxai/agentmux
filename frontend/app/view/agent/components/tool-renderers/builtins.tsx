// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The built-in result renderers: one per coarse tool kind, plus the
 * catch-all. Moved out of ToolOverlayLog.tsx
 * (SPEC_AGENT_PANE_TOOL_DESCRIPTORS_2026_09_26.md §2.5), which ended the
 * DispatchCard ↔ ToolOverlayLog import cycle: DispatchCard's no-match
 * fallback delegates to renderAgent / renderTask / renderWorkflow here.
 * Registered through tool-renderers/index.ts, like every other renderer.
 */

import { Markdown } from "@/app/element/markdown";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Show, type JSX } from "solid-js";
import type { ResultFileFacts } from "../../providers/claude-translator";
import type { ToolNode } from "../../types";
import { BashOutputViewer } from "../BashOutputViewer";
import { CompactResult } from "../CompactResult";
import { DiffViewer } from "../DiffViewer";
import { formatCodePreview, formatMarkdownPreview, formatReadPreview } from "../dedent";
import { detectLanguage } from "../detectLanguage";
import { HighlightedCode } from "../HighlightedCode";
import { OutputHiddenMarker } from "../OutputHiddenMarker";
import { ResultImages, resultImagesOf } from "../ResultImages";
import { capText, MAX_TOOL_OUTPUT_LINES, type CappedText } from "../output-cap";
import { terminalText } from "../terminal-text";
import { fileRangeOf, formatFileRangeLong } from "../../tool-meta/file-range";
import { anyTool, byKind, type ToolRendererEntry } from "./registry";

// Per-tool result renderers, registered through BUILTIN_RENDERERS so the open-ended tool universe can be routed by name/shape rather than
// a closed switch — these are the built-in (coarse-kind) entries. The bodies are
// the former `switch` arms verbatim; behavior is unchanged. See
// SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md (Phase 1).

function renderEdit(node: ToolNode): JSX.Element {
    const range = fileRangeOf(node);
    return (
        <div class="agent-tool-edit">
            <Show when={range}>
                <div class="agent-tool-edit-range">
                    <span class="agent-tool-read-range">{formatFileRangeLong(range!)}</span>
                </div>
            </Show>
            <DiffViewer params={node.params as any} result={node.result as any} status={node.status} />
        </div>
    );
}

function renderBash(node: ToolNode): JSX.Element {
    return <BashOutputViewer result={node.result as any} />;
}

const isMarkdownPath = (path: string): boolean => path.endsWith(".md") || path.endsWith(".mdx");

/**
 * The file-preview pipeline Read and Write share
 * (SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §4): a markdown file renders
 * as markdown, anything else as highlighted code, then the hidden-lines
 * marker for a head-capped file. Each caller formats the text its own way
 * (see renderRead / renderWrite for why they differ); `classPrefix` keeps
 * each tool's class names.
 */
function FilePreview(props: {
    path: string;
    /** The head-capped raw text (language sniffing reads its first line). */
    capped: CappedText;
    /** What the highlighted-code view shows. */
    code: string;
    /** What the markdown view shows. */
    markdown: string;
    classPrefix: "agent-tool-read" | "agent-tool-write";
}): JSX.Element {
    return (
        <>
            <Show
                when={isMarkdownPath(props.path)}
                fallback={
                    <HighlightedCode
                        code={props.code}
                        // Language detection reads the RAW (non-dedented) first
                        // line — shebang/content sniffing should see the file
                        // as-is; only the displayed text is dedented.
                        lang={detectLanguage(props.path, props.capped.text.split("\n")[0])}
                        class={`${props.classPrefix}-content`}
                    />
                }
            >
                <div class={`${props.classPrefix}-content ${props.classPrefix}-md`}>
                    {/* scrollable={false}, same as MarkdownBlock: this
                        preview lives inside the virtualized document,
                        which owns the scroll. `scrollable` defaults to
                        true, and each mount then constructs an
                        OverlayScrollbars instance — getComputedStyle +
                        scrollLeft probes that each force a layout of the
                        whole pane. Measured at 46% of `flushPendingNodes`
                        under load (ANALYSIS_AGENT_PANE_FLUSH_REMOUNT_CHURN_2026_09_23.md §2). */}
                    <Markdown text={props.markdown} scrollable={false} />
                </div>
            </Show>
            <Show when={props.capped.hiddenLines > 0}>
                <OutputHiddenMarker hidden={props.capped.hiddenLines} noun="line" from="head" />
            </Show>
        </>
    );
}

/**
 * Claude Code appends notes for the model to the end of a Read's text
 * (`<system-reminder>…</system-reminder>`, e.g. the token cap's "PARTIAL
 * view" note). The range line already says what they say, and inside the code
 * they broke the gutter, so the preview shows only the file. Only trailing
 * notes are taken off: a file may mention the tag itself.
 */
const TRAILING_NOTE_RE = /(?:^|\n)<system-reminder>[\s\S]*?<\/system-reminder>\s*$/;

export function withoutTrailingNotes(text: string): string {
    let out = text;
    for (let m = TRAILING_NOTE_RE.exec(out); m; m = TRAILING_NOTE_RE.exec(out)) out = out.slice(0, m.index);
    return out.trimEnd();
}

/**
 * One line for a Read that returned something other than the file's text: an
 * image (its type, dimensions and size), a PDF (its size and the pages asked
 * for), or a file unchanged since the last Read. Null for a text Read.
 */
export function readFactsLine(node: ToolNode): string | null {
    const file = (node.result as { file?: ResultFileFacts } | undefined)?.file;
    if (file?.kind === "unchanged") return "unchanged since the last Read";
    const images = resultImagesOf(node.result);
    const parts: string[] = [];
    if (file?.kind === "pdf") {
        parts.push("PDF");
        const pages = (node.params as { pages?: unknown } | undefined)?.pages;
        if (typeof pages === "string" && pages.trim() !== "") parts.push(`pages ${pages.trim()}`);
    } else if (images.length > 0 || file?.kind === "image") {
        const type = images[0]?.mediaType.replace(/^image\//i, "").replace(/\+xml$/i, "").toUpperCase();
        parts.push(type ? `${type} image` : "image");
        if (file?.width && file?.height) parts.push(`${file.width} × ${file.height}`);
    } else {
        return null;
    }
    if (file?.size != null) parts.push(formatBytes(file.size));
    return parts.join(" · ");
}

function renderRead(node: ToolNode): JSX.Element {
    const filePath = (node.params as any).file_path ?? "";
    const raw: string | undefined = (node.result as any)?.content;
    const facts = readFactsLine(node);
    const file = (node.result as { file?: ResultFileFacts } | undefined)?.file;
    const images = resultImagesOf(node.result);
    // An image, PDF or unchanged-file Read has a note for the model as its
    // text, not the file: the facts line replaces it.
    const content = facts ? undefined : raw ? withoutTrailingNotes(raw) : raw;
    // Head-cap file content (read top-down) so a huge Read can't bloat
    // the conversation DOM; HighlightedCode stays simple (it injects
    // innerHTML, so capping here is cleaner than inside it).
    const capped = content ? capText(content, MAX_TOOL_OUTPUT_LINES, "head") : null;
    // Dedent (common to every VISIBLE line — computed after capping so a
    // deeper hidden tail can't reduce the dedent of what's actually shown,
    // SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md §3.2.1), narrow the remaining
    // relative indentation to 2 columns per level, and re-emit Claude Code's
    // "<N>\t" line-number gutter right-aligned at a fixed width. That last
    // part removes the tab, and with it the sideways step the code column
    // took at every digit-count boundary (9→10, 999→1000) — see dedent.ts's
    // module header and docs/analysis/tool-preview-indentation-and-wrapping-2026-09-02.md.
    const preview = capped ? formatReadPreview(capped.text) : null;
    const range = fileRangeOf(node);
    return (
        <div class="agent-tool-read">
            <div class="agent-tool-file-path-row">
                <Show when={range}>
                    <span class="agent-tool-read-range">{formatFileRangeLong(range!)}</span>
                </Show>
                <span class="agent-tool-file-path">{filePath}</span>
            </div>
            <Show when={facts}>
                <div class="agent-tool-read-facts">
                    <span>{facts}</span>
                    <Show when={file?.kind === "pdf" && filePath}>
                        <button
                            type="button"
                            class="agent-tool-read-open"
                            title="Open with the default app"
                            onClick={() => void RpcApi.FsOpenCommand(TabRpcClient, { path: filePath }).catch(() => {})}
                        >
                            Open
                        </button>
                        <button
                            type="button"
                            class="agent-tool-read-open"
                            title="Show the file in the OS file manager"
                            onClick={() => window.api?.revealInFileExplorer(filePath)}
                        >
                            Show in folder
                        </button>
                    </Show>
                </div>
            </Show>
            <Show when={images.length > 0}>
                <ResultImages images={images} path={filePath || undefined} width={file?.width} height={file?.height} />
            </Show>
            <Show
                when={capped}
                fallback={
                    <Show when={node.result && !facts}>
                        <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
                    </Show>
                }
            >
                {/* Markdown gets `body`, not `withGutter`: a line-number column
                    is meaningless in rendered markdown and actively corrupts it
                    (a "1\t# Title" line is not a heading;
                    SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md §2.1). */}
                <FilePreview
                    path={filePath}
                    capped={capped!}
                    code={preview!.withGutter}
                    markdown={preview!.body}
                    classPrefix="agent-tool-read"
                />
            </Show>
        </div>
    );
}

function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function renderWrite(node: ToolNode): JSX.Element {
    const filePath = (node.params as any).file_path ?? "";
    const content: string | undefined = (node.params as any).content;
    const bytes: number | undefined = (node.result as any)?.bytesWritten;
    const capped = content ? capText(content, MAX_TOOL_OUTPUT_LINES, "head") : null;
    const range = fileRangeOf(node);
    return (
        <div class="agent-tool-write">
            <div class="agent-tool-file-path-row">
                <Show when={range}>
                    <span class="agent-tool-read-range">{formatFileRangeLong(range!)}</span>
                </Show>
                <span class="agent-tool-file-path">{filePath}</span>
                <Show when={bytes != null}>
                    <span class="agent-tool-write-bytes">{formatBytes(bytes!)}</span>
                </Show>
            </div>
            <Show when={capped} fallback={<div class="agent-tool-write-info">No content written.</div>}>
                <FilePreview
                    path={filePath}
                    capped={capped!}
                    // Plain dedent + narrow, NOT the Read-specific gutter-aware
                    // variant (SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md §3.2.3) —
                    // Write content has no CLI-added "<N>\t" line-number prefix,
                    // so formatReadPreview's numbered heuristic would misfire on
                    // a genuine tab-delimited file (TSV/BED/GTF) whose every
                    // non-blank line happens to start with digits+tab, silently
                    // dropping that real leading column. Its dedent half is a
                    // no-op for the common already-flush case (a whole file
                    // starts at column 0), which is why the narrowing half is
                    // the part that fires on a Write.
                    code={formatCodePreview(capped!.text)}
                    // Markdown is indentation-sensitive — four leading spaces
                    // are a code block, and rescaling them to two turns it into
                    // prose. Dedent only for that path (codex P2 on PR #2958);
                    // `formatMarkdownPreview` documents why.
                    markdown={formatMarkdownPreview(capped!.text)}
                    classPrefix="agent-tool-write"
                />
            </Show>
        </div>
    );
}

// No "Pattern:" line: the row header already shows the pattern
// (tool-header.ts). SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.5.
function renderSearch(node: ToolNode): JSX.Element {
    return (
        <div class="agent-tool-search">
            <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
        </div>
    );
}

// Exported (not just module-local) so `DispatchCard.tsx`'s no-match fallback
// can delegate to the SAME per-kind rendering these built-ins already do
// (the report as markdown, no-result gating) instead of a bare
// `CompactResult` call that loses both — reagent/codex P1 on PR #2676: a
// still-running unmatched Agent/Task call was showing raw "No output" and a
// completed one was losing its description entirely, guaranteed to trigger
// on the Agent History tab (which always falls back to CompactResult there).
export function renderAgent(node: ToolNode): JSX.Element {
    // A subagent's report is markdown; render it as such rather than as a
    // compact one-liner (SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.6).
    // No description line: the row header already shows it (tool-header.ts),
    // running or finished.
    // Head-capped like a Read preview: the panel's max-height bounds what's
    // visible, not the DOM, and a subagent report can be very long.
    const text = terminalText(node.result);
    const report = text ? capText(text, MAX_TOOL_OUTPUT_LINES, "head") : null;
    return (
        <div class="agent-tool-agent">
            <Show
                when={report}
                fallback={
                    <Show when={node.result}>
                        <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
                    </Show>
                }
            >
                <div class="agent-tool-agent-report">
                    {/* scrollable={false} — see renderRead's markdown branch. */}
                    <Markdown text={report!.text} scrollable={false} />
                </div>
                <Show when={report!.hiddenLines > 0}>
                    <OutputHiddenMarker hidden={report!.hiddenLines} noun="line" from="head" />
                </Show>
            </Show>
        </div>
    );
}

export function renderTask(node: ToolNode): JSX.Element {
    return (
        <div class="agent-tool-task">
            <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
        </div>
    );
}

export function renderWorkflow(node: ToolNode): JSX.Element {
    // The row header shows the title, or the description when there's no
    // title; repeat the description here only when the header shows the title.
    const params = node.params as any;
    const extraDesc = params.title && params.description && params.description !== params.title ? params.description : null;
    return (
        <div class="agent-tool-workflow">
            <Show when={extraDesc}>
                <div class="agent-tool-agent-desc">{extraDesc}</div>
            </Show>
            <Show when={node.result}>
                <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
            </Show>
        </div>
    );
}

export function renderCompactDefault(node: ToolNode): JSX.Element {
    // An image a tool returned (an MCP screenshot) shows as the image, with
    // the result's text below it.
    const images = resultImagesOf(node.result);
    if (images.length === 0) return <CompactResult tool={node.tool} params={node.params as Record<string, unknown>} result={node.result} />;
    const text = (node.result as { content?: unknown }).content;
    return (
        <div class="agent-tool-media-result">
            <ResultImages images={images} />
            <Show when={typeof text === "string" && text.trim() !== ""}>
                <CompactResult tool={node.tool} params={node.params as Record<string, unknown>} result={{ content: text }} />
            </Show>
        </div>
    );
}

// Built-ins sit at priority 0 and the catch-all below everything; rich,
// name- or shape-matched renderers (WebSearch, mcp__* tools, DispatchCard, …)
// sit above (see registry.ts for the priority convention).
export const BUILTIN_RENDERERS: readonly ToolRendererEntry[] = [
    { priority: 0, label: "builtin:Edit", match: byKind("Edit"), render: renderEdit },
    { priority: 0, label: "builtin:Bash", match: byKind("Bash"), render: renderBash },
    { priority: 0, label: "builtin:Read", match: byKind("Read"), render: renderRead },
    { priority: 0, label: "builtin:Write", match: byKind("Write"), render: renderWrite },
    { priority: 0, label: "builtin:Search", match: byKind("Grep", "Glob"), render: renderSearch },
    { priority: 0, label: "builtin:Agent", match: byKind("Agent"), render: renderAgent },
    { priority: 0, label: "builtin:Task", match: byKind("Task"), render: renderTask },
    { priority: 0, label: "builtin:Workflow", match: byKind("Workflow"), render: renderWorkflow },
    { priority: -Infinity, label: "builtin:default", match: anyTool, render: renderCompactDefault },
];
