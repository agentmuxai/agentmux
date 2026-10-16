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
import { Button } from "@/app/element/ui";
import { tokensLabel, tokensTitle } from "@/app/view/native-memory/memory-tokens";
import { growthLabel, growthOver, sparklinePoints, versionDelta } from "./memory-history-tokens";

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
 *  agent can't fake the way it can `source`. The Memory pane itself is implied by
 *  a "Human" source, so it isn't repeated. */
function writerLabel(v: MemoryVersionMeta): string | null {
    if (!v.written_by || v.written_by === "armory-ui") return null;
    return `by ${v.written_by}`;
}

/** The size over time above the list: now, growth over 7 and 30 days, and a
 *  sparkline of every version's tokens (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.5). */
function HistorySize<V extends MemoryVersionMeta>(props: { versions: V[] }): JSX.Element {
    const latest = () => props.versions[0]?.tokens_est;
    const growth = (days: number) => growthOver(props.versions, days, Date.now());
    const points = () => sparklinePoints(props.versions, 120, 18);
    return (
        <Show when={latest() != null}>
            <div class="memory-history-size" data-testid="memory-history-size">
                <span title={tokensTitle(latest() ?? 0, props.versions[0]?.size_bytes)}>
                    Now <strong>{tokensLabel(latest() ?? 0)}</strong>
                </span>
                <span>7 days: {growthLabel(growth(7) ?? 0)}</span>
                <span>30 days: {growthLabel(growth(30) ?? 0)}</span>
                <Show when={points()}>
                    <svg class="memory-history-sparkline" viewBox="0 0 120 18" width="120" height="18" aria-hidden="true">
                        <polyline points={points()} />
                    </svg>
                </Show>
            </div>
        </Show>
    );
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
                    <HistorySize versions={model.versionsAtom()} />
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
                                                <Show when={v.tokens_est != null}>
                                                    <span
                                                        class="native-memory-history-item-tokens"
                                                        title={tokensTitle(v.tokens_est ?? 0, v.size_bytes)}
                                                    >
                                                        {tokensLabel(v.tokens_est ?? 0)}
                                                        {versionDelta(model.versionsAtom(), i()) ? ` · ${versionDelta(model.versionsAtom(), i())}` : ""}
                                                    </span>
                                                </Show>
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
                                                    <Button
                                                        density="compact"
                                                        class="native-memory-history-btn"
                                                        disabled={model.revertingAtom()}
                                                        onClick={() => setConfirmingRevert(null)}
                                                    >
                                                        Cancel
                                                    </Button>
                                                    <Button
                                                        density="compact"
                                                        tone="accent"
                                                        class="native-memory-history-btn native-memory-history-btn-primary"
                                                        disabled={model.revertingAtom() || props.revertDisabled}
                                                        onClick={() => {
                                                            setConfirmingRevert(null);
                                                            void model.revertTo(v.id);
                                                        }}
                                                    >
                                                        {model.revertingAtom() ? "Reverting…" : "Confirm revert"}
                                                    </Button>
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
                        <Button density="compact" class="native-memory-history-btn" onClick={() => model.clearDiffSelection()}>
                            Clear selection
                        </Button>
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
