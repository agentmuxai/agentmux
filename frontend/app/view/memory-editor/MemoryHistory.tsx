// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryHistory — version list, two-version diff and revert for one memory,
 * over a `MemoryHistoryModel` (whose data source decides whether that is a
 * native memory file or a Global Memory entry). The history half of what was
 * `NativeMemoryHistoryPanel`; the content half is `MemoryContent`.
 * docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3.
 */

import { createSignal, For, Show, type JSX } from "solid-js";
import { diffLineClass, diffLines } from "./line-diff";
import { sourceLabel, sourceWarning, type MemoryHistoryModel, type MemoryVersionMeta } from "./memory-history-model";
import "./memory-history.scss";

interface MemoryHistoryProps<V extends MemoryVersionMeta> {
    model: MemoryHistoryModel<V>;
    /** Disables Revert (e.g. while the version is being saved). */
    revertDisabled?: boolean;
}

function formatTimestamp(ms: number): string {
    if (!ms) return "unknown time";
    return new Date(ms).toLocaleString(undefined, {
        year: "numeric", month: "short", day: "numeric",
        hour: "2-digit", minute: "2-digit",
    });
}

/** "by <writer>" for Global Memory versions — the trusted writer, which an
 *  agent can't fake the way it can `source`. The Armory itself is implied by
 *  a "Human" source, so it isn't repeated. */
function writerLabel(v: MemoryVersionMeta): string | null {
    if (!v.written_by || v.written_by === "armory-ui") return null;
    return `by ${v.written_by}`;
}

export function MemoryHistory<V extends MemoryVersionMeta>(props: MemoryHistoryProps<V>): JSX.Element {
    const model = props.model;
    const [confirmingRevert, setConfirmingRevert] = createSignal<string | null>(null);

    return (
        <div class="native-memory-history memory-history" data-testid="memory-history">
            <Show when={model.errorAtom()}>
                <div class="native-memory-history-error">{model.errorAtom()}</div>
            </Show>

            <Show
                when={!model.loadingAtom() || model.versionsAtom().length > 0}
                fallback={<div class="native-memory-history-loading">Loading history…</div>}
            >
                <Show
                    when={model.versionsAtom().length > 0}
                    fallback={<div class="native-memory-history-empty">No recorded versions yet.</div>}
                >
                    <div class="native-memory-history-hint">
                        Select two versions to compare, or revert directly to one.
                    </div>
                    <ul class="native-memory-history-list">
                        <For each={model.versionsAtom()}>
                            {(v, i) => {
                                const warning = sourceWarning(v);
                                const writer = writerLabel(v);
                                const selected = () => model.diffSelectionAtom().includes(v.id);
                                return (
                                    <li
                                        class="native-memory-history-item"
                                        classList={{ "is-selected": selected(), "is-latest": i() === 0 }}
                                    >
                                        <label class="native-memory-history-item-select">
                                            <input
                                                type="checkbox"
                                                checked={selected()}
                                                onChange={() => model.toggleDiffSelection(v.id)}
                                            />
                                        </label>
                                        <div class="native-memory-history-item-body">
                                            <div class="native-memory-history-item-meta">
                                                <span
                                                    class="native-memory-history-item-source"
                                                    classList={{ "is-warning": warning !== null }}
                                                >
                                                    {sourceLabel(v.source)}
                                                </span>
                                                <Show when={writer}>
                                                    <span class="native-memory-history-item-time">{writer}</span>
                                                </Show>
                                                <span class="native-memory-history-item-time">
                                                    {formatTimestamp(v.created_at)}
                                                </span>
                                                <Show when={i() === 0}>
                                                    <span class="native-memory-history-item-badge">current</span>
                                                </Show>
                                            </div>
                                            <Show when={warning}>
                                                <div class="native-memory-history-item-warning" title={warning ?? undefined}>
                                                    ⚠ {warning}
                                                </div>
                                            </Show>
                                        </div>
                                        <Show when={i() !== 0}>
                                            <Show
                                                when={confirmingRevert() === v.id}
                                                fallback={
                                                    <button
                                                        class="native-memory-history-revert-btn"
                                                        disabled={model.revertingAtom() || props.revertDisabled}
                                                        onClick={() => setConfirmingRevert(v.id)}
                                                    >
                                                        Revert to this
                                                    </button>
                                                }
                                            >
                                                <div class="native-memory-history-revert-confirm">
                                                    <span>Restore this content as a new version?</span>
                                                    <button
                                                        class="native-memory-history-btn"
                                                        disabled={model.revertingAtom()}
                                                        onClick={() => setConfirmingRevert(null)}
                                                    >
                                                        Cancel
                                                    </button>
                                                    <button
                                                        class="native-memory-history-btn native-memory-history-btn-primary"
                                                        disabled={model.revertingAtom() || props.revertDisabled}
                                                        onClick={() => {
                                                            setConfirmingRevert(null);
                                                            void model.revertTo(v.id);
                                                        }}
                                                    >
                                                        {model.revertingAtom() ? "Reverting…" : "Confirm revert"}
                                                    </button>
                                                </div>
                                            </Show>
                                        </Show>
                                    </li>
                                );
                            }}
                        </For>
                    </ul>
                </Show>
            </Show>

            <Show when={model.diffSelectionAtom().length === 2}>
                <div class="native-memory-history-diff">
                    <div class="native-memory-history-diff-header">
                        <span>Diff</span>
                        <button class="native-memory-history-btn" onClick={() => model.clearDiffSelection()}>
                            Clear selection
                        </button>
                    </div>
                    <Show
                        when={!model.diffLoadingAtom()}
                        fallback={<div class="native-memory-history-loading">Loading diff…</div>}
                    >
                        <pre class="native-memory-history-diff-body">
                            <For each={diffLines(model.diffTextAtom() ?? "")}>
                                {(line) => <div class={diffLineClass(line)}>{line || " "}</div>}
                            </For>
                        </pre>
                    </Show>
                </div>
            </Show>
        </div>
    );
}
