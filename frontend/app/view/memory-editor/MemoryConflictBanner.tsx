// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The "changed since you started editing" banner
 * (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4). Renders only while the
 * draft model holds a conflict; the draft itself is never touched by it.
 *
 *   View change   — a diff of the draft's base → what is saved now;
 *   Keep editing  — dismiss; the draft keeps its ORIGINAL base, so a save
 *                   still can't silently overwrite the other change;
 *   Save anyway   — explicit overwrite (rebase onto what's saved, then save);
 *   Discard my edits.
 */

import { createSignal, For, Show, type JSX } from "solid-js";
import { diffLineClass, diffLines, lineDiff } from "./line-diff";
import type { MemoryDraftModel } from "./memory-draft-model";

interface MemoryConflictBannerProps<T> {
    model: MemoryDraftModel<T>;
    /** Text form of a value, for the View change diff. */
    toText: (value: T) => string;
    /** What's being edited, e.g. "this file" / "this memory". */
    noun: string;
    /** After Discard, so the caller can re-show the saved content. */
    onDiscarded?: () => void;
    /** Whether "Save anyway" can re-create something deleted meanwhile
     *  (a memory file can; a Global Memory entry can't). Default true. */
    canRecreate?: boolean;
}

export function MemoryConflictBanner<T>(props: MemoryConflictBannerProps<T>): JSX.Element {
    const [showDiff, setShowDiff] = createSignal(false);

    const message = () => {
        const c = props.model.conflictAtom();
        if (!c) return "";
        if (c.current === null) return `${capitalize(props.noun)} was deleted since you started editing.`;
        return c.reason === "refused"
            ? `Not saved — ${props.noun} changed since you started editing.`
            : `${capitalize(props.noun)} changed since you started editing.`;
    };

    const diffText = () => {
        const c = props.model.conflictAtom();
        const base = props.model.baseAtom();
        if (!c || c.current === undefined) return "";
        const from = base === null ? "" : props.toText(base);
        const to = c.current === null ? "" : props.toText(c.current);
        return lineDiff(from, to);
    };

    return (
        <Show when={props.model.conflictAtom()}>
            {(conflict) => (
                <div class="memory-conflict-banner" role="alert" data-testid="memory-conflict-banner">
                    <span>{message()} Your draft is kept.</span>
                    <div class="memory-conflict-banner-actions">
                        <button
                            type="button"
                            class="memory-editor-btn"
                            classList={{ "is-active": showDiff() }}
                            disabled={conflict().current === undefined}
                            onClick={() => setShowDiff(!showDiff())}
                        >
                            View change
                        </button>
                        <button
                            type="button"
                            class="memory-editor-btn is-primary"
                            onClick={() => {
                                setShowDiff(false);
                                props.model.keepEditing();
                            }}
                        >
                            Keep editing
                        </button>
                        <Show when={props.canRecreate !== false || conflict().current !== null}>
                            <button
                                type="button"
                                class="memory-editor-btn"
                                disabled={conflict().current === undefined || props.model.savingAtom()}
                                title="Replace the saved version with your draft"
                                onClick={() => void props.model.overwrite()}
                            >
                                Save anyway
                            </button>
                        </Show>
                        <button
                            type="button"
                            class="memory-editor-btn is-danger"
                            onClick={() => {
                                props.model.discard();
                                props.onDiscarded?.();
                            }}
                        >
                            Discard my edits
                        </button>
                    </div>
                    <Show when={showDiff()}>
                        <pre class="memory-conflict-banner-diff" data-testid="memory-conflict-diff">
                            <For each={diffLines(diffText())}>
                                {(line) => <div class={diffLineClass(line)}>{line || " "}</div>}
                            </For>
                        </pre>
                    </Show>
                </div>
            )}
        </Show>
    );
}

function capitalize(s: string): string {
    return s ? s[0].toUpperCase() + s.slice(1) : s;
}
