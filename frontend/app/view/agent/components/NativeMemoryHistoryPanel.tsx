// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NativeMemoryHistoryPanel — version history / diff / revert for one memory
 * file. Shared component, mounted from two places (props are the only
 * difference between them, never the underlying data):
 *   - AgentNativeMemoryModal (Stash "Memory" tab) — agentId fixed to the
 *     pane's own agent.
 *   - NativeMemoryManager (Armory "Native Memory" tab) — agentId comes from
 *     an agent picker.
 * Both read the identical agent:memory:history/diff/revert RPCs — one
 * source of truth, two entry points. See
 * docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.3.
 *
 * Remounts per (agentId, filename) — this component does not itself react
 * to those props changing after mount; callers that let the user switch
 * agent/file (e.g. Armory's picker) must force a remount (e.g. a keyed
 * <Show>) rather than relying on this component to re-fetch internally.
 */

import { For, Show, createSignal, onCleanup, type JSX } from "solid-js";
import { NativeMemoryHistoryModel, sourceLabel, sourceWarning } from "../native-memory-history-model";
import "./NativeMemoryHistoryPanel.scss";

interface NativeMemoryHistoryPanelProps {
    agentId: string;
    filename: string;
    /** Called with the restored content immediately after a successful
     *  revert, so a caller showing "current content" elsewhere (e.g.
     *  AgentNativeMemoryModal's own view pane) can refresh without a
     *  separate read_file round trip of its own. */
    onContentReverted?: (content: string) => void;
}

function formatTimestamp(ms: number): string {
    if (!ms) return "unknown time";
    return new Date(ms).toLocaleString(undefined, {
        year: "numeric", month: "short", day: "numeric",
        hour: "2-digit", minute: "2-digit",
    });
}

/** One line of `NativeMemoryDiffResult.diff`, tagged for styling. */
function diffLineClass(line: string): string {
    if (line.startsWith("+ ")) return "native-memory-diff-line is-added";
    if (line.startsWith("- ")) return "native-memory-diff-line is-removed";
    return "native-memory-diff-line";
}

export const NativeMemoryHistoryPanel = (props: NativeMemoryHistoryPanelProps): JSX.Element => {
    const model = new NativeMemoryHistoryModel(props.agentId, props.filename);
    onCleanup(() => model.dispose());
    if (props.onContentReverted) {
        model.onReverted = props.onContentReverted;
    }

    const [confirmingRevert, setConfirmingRevert] = createSignal<string | null>(null);

    return (
        <div class="native-memory-history">
            <Show when={model.errorAtom()}>
                <div class="native-memory-history-error">{model.errorAtom()}</div>
            </Show>

            <Show
                when={!model.loadingAtom()}
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
                                                        disabled={model.revertingAtom()}
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
                                                        disabled={model.revertingAtom()}
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
                            <For each={(model.diffTextAtom() ?? "").split("\n").filter((_, idx, arr) => idx < arr.length - 1 || arr[idx] !== "")}>
                                {(line) => <div class={diffLineClass(line)}>{line || " "}</div>}
                            </For>
                        </pre>
                    </Show>
                </div>
            </Show>
        </div>
    );
};

NativeMemoryHistoryPanel.displayName = "NativeMemoryHistoryPanel";
