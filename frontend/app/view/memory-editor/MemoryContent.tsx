// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryContent — a memory's content, read-only or as an editor, filling the
 * bottom region of a `PinnedEditorLayout`. The content half of what was
 * `NativeMemoryHistoryPanel` (the history half is `MemoryHistory`), and the
 * one place the three memory surfaces render their view/edit body.
 * docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3/§2.4.
 */

import { Show, type JSX } from "solid-js";
import { Markdown } from "@/app/element/markdown";
import { showTextInputContextMenu } from "@/app/store/contextmenu";

interface MemoryContentProps {
    /** Saved content; `null` = not loaded (or failed — see `error`). */
    content: string | null;
    loading?: boolean;
    error?: string | null;
    /** When set, render an editor on `draft` instead of the content. */
    editing: boolean;
    draft?: string;
    onDraftInput?: (value: string) => void;
    /** "markdown" renders the read-only view as Markdown (the Armory);
     *  "plain" as preformatted text (the Stash, which always has). */
    view?: "markdown" | "plain";
    placeholder?: string;
    emptyText?: string;
    textareaLabel?: string;
}

export function MemoryContent(props: MemoryContentProps): JSX.Element {
    return (
        <div class="memory-content" data-testid="memory-content">
            <Show
                when={props.editing}
                fallback={
                    <>
                        <Show when={props.error}>
                            <div class="memory-content-error">{props.error}</div>
                        </Show>
                        <Show when={props.loading && props.content === null}>
                            <p class="memory-content-status">Loading…</p>
                        </Show>
                        <Show when={props.content !== null}>
                            <Show
                                when={props.content !== ""}
                                fallback={<p class="memory-content-status">{props.emptyText ?? "Empty."}</p>}
                            >
                                <Show
                                    when={props.view !== "plain"}
                                    fallback={<pre class="memory-content-pre">{props.content}</pre>}
                                >
                                    <div class="memory-content-markdown">
                                        <Markdown
                                            text={props.content ?? ""}
                                            scrollable={true}
                                            nativeScrollbar={true}
                                            contentClass="memory-content-markdown-content"
                                        />
                                    </div>
                                </Show>
                            </Show>
                        </Show>
                    </>
                }
            >
                <textarea
                    class="memory-content-textarea"
                    aria-label={props.textareaLabel ?? "Memory content"}
                    value={props.draft ?? ""}
                    onInput={(e) => props.onDraftInput?.(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder={props.placeholder}
                    spellcheck={false}
                />
            </Show>
        </div>
    );
}
