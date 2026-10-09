// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PreviewLines — the one renderer for every tool preview's text
 * (docs/reports/REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §5.2).
 *
 * Takes a {@link PreviewDoc} from `preview-text/` and draws one block per line:
 * the line number or diff marker in its own box, then the text. Because the
 * gutter and marker aren't part of the text, they take no part in tab stops,
 * wrapping or selection, and a diff line's background spans the whole line.
 *
 * Code and diffs are highlighted per line once Shiki resolves; until then (and
 * for languages Shiki can't load) the plain text shows, in the same layout, so
 * nothing moves when the colours arrive. The `mode` decides whether long lines
 * scroll sideways (code, command output) or wrap (prose).
 */

import { LinkifiedText } from "@/app/element/linkified-text";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, For, Index, onCleanup, Show, type JSX } from "solid-js";
import { highlightDoc, type CodeToken } from "../preview-text/highlight";
import type { PreviewDoc, PreviewLine, PreviewMode } from "../preview-text/types";
import { OutputHiddenMarker } from "./OutputHiddenMarker";

/** How each kind shows unless the caller says otherwise
 *  (REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §8). */
const DEFAULT_MODE: Record<PreviewDoc["kind"], PreviewMode> = {
    code: "scroll",
    diff: "scroll",
    output: "scroll",
    json: "scroll",
    prose: "wrap",
};

const MARKER_CLASS: Record<string, string> = { "+": "add", "-": "del", " ": "ctx", "@@": "hunk" };

interface PreviewLinesProps {
    doc: PreviewDoc;
    mode?: PreviewMode;
    /** Turn URLs in unhighlighted lines into links. */
    linkify?: boolean;
    class?: string;
}

export function PreviewLines(props: PreviewLinesProps): JSX.Element {
    const [tokens, setTokens] = createSignal<(CodeToken[] | null)[] | null>(null);
    createEffect(() => {
        const doc = props.doc;
        setTokens(null);
        if (doc.kind !== "code" && doc.kind !== "diff") return;
        let cancelled = false;
        onCleanup(() => (cancelled = true));
        void highlightDoc(doc).then((t) => {
            if (!cancelled) setTokens(t);
        });
    });

    const mode = () => props.mode ?? DEFAULT_MODE[props.doc.kind];
    // Memos: read once per line below, and the gutter width walks every line.
    const gutterWidth = createMemo(() => {
        let max = 0;
        for (const l of props.doc.lines) if (l.number != null) max = Math.max(max, String(l.number).length);
        return max;
    });
    const hasMarkers = createMemo(() => props.doc.kind === "diff");

    return (
        <div
            class={clsx("agent-preview", `agent-preview--${mode()}`, props.class)}
            data-kind={props.doc.kind}
            style={gutterWidth() > 0 ? { "--agent-preview-gutter": `${gutterWidth()}ch` } : undefined}
        >
            <Show when={props.doc.hidden?.from === "tail"}>
                <OutputHiddenMarker hidden={props.doc.hidden!.count} noun="line" from="tail" />
            </Show>
            <div class="agent-preview-lines">
                <Index each={props.doc.lines}>
                    {(line, i) => (
                        <div
                            class={clsx(
                                "agent-preview-line",
                                line().marker && `agent-preview-line--${MARKER_CLASS[line().marker!]}`,
                                line().stream && `agent-preview-line--${line().stream}`
                            )}
                        >
                            <Show when={gutterWidth() > 0}>
                                <span class="agent-preview-gutter" aria-hidden="true">
                                    {line().number ?? ""}
                                </span>
                            </Show>
                            <Show when={hasMarkers() && line().marker !== "@@"}>
                                <span class="agent-preview-marker" aria-hidden="true">
                                    {line().marker}
                                </span>
                            </Show>
                            <span class="agent-preview-text">
                                <LineText line={line()} tokens={tokens()?.[i] ?? null} linkify={props.linkify} />
                            </span>
                        </div>
                    )}
                </Index>
            </div>
            <Show when={props.doc.hidden?.from === "head"}>
                <OutputHiddenMarker hidden={props.doc.hidden!.count} noun="line" from="head" />
            </Show>
        </div>
    );
}

function LineText(props: { line: PreviewLine; tokens: CodeToken[] | null; linkify?: boolean }): JSX.Element {
    return (
        <Show
            when={props.tokens}
            fallback={
                <Show
                    when={props.line.spans}
                    fallback={
                        <Show when={props.linkify} fallback={props.line.text}>
                            <LinkifiedText text={props.line.text} />
                        </Show>
                    }
                >
                    <For each={props.line.spans}>
                        {(s) => (s.classes ? <span class={s.classes}>{s.text}</span> : s.text)}
                    </For>
                </Show>
            }
        >
            <For each={props.tokens}>{(t) => <span style={tokenStyle(t)}>{t.content}</span>}</For>
        </Show>
    );
}

function tokenStyle(t: CodeToken): JSX.CSSProperties | undefined {
    const fs = t.fontStyle ?? 0;
    if (!t.color && !fs) return undefined;
    return {
        ...(t.color ? { color: t.color } : {}),
        ...(fs & 1 ? { "font-style": "italic" } : {}),
        ...(fs & 2 ? { "font-weight": "bold" } : {}),
        ...(fs & 4 ? { "text-decoration": "underline" } : {}),
    };
}
