// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PersistentShellBlock — compact document row for a long-running shell process.
 *
 * Displays as a single colored line in the conversation (status color on the
 * left border, sigil, title, elapsed timer, live-tail of last output).
 * Click toggles an inline log panel showing all streamed output.
 *
 * Spec: docs/specs/SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md
 */

import clsx from "clsx";
import { For, Show, createMemo, createSignal, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatElapsedClock, formatExactTime, formatTimeAgo } from "@/util/format-time";
import { useNodePeek } from "../hooks/useNodePeek";
import { capChars, createChunkCapper, createSpinnerCollapser, MAX_TOOL_OUTPUT_LINES } from "./output-cap";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { LinkifiedText } from "@/app/element/linkified-text";
import { PeekOverlay } from "./PeekOverlay";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { ShellNode, ToolLogChunk } from "../types";

interface PersistentShellBlockProps {
    node: ShellNode;
    pinned: boolean;
    onTogglePin: () => void;
}

const KIND_CLASS: Record<string, string> = {
    stdout: "agent-tool-log-line--stdout",
    stderr: "agent-tool-log-line--stderr",
    system: "agent-tool-log-line--system",
};

export const PersistentShellBlock = (props: PersistentShellBlockProps): JSX.Element => {
    const tick = useTick(1000);

    const elapsed = createMemo(() => {
        const end = props.node.exitedAt ?? (tick(), Date.now());
        return formatElapsedClock(end - props.node.spawnedAt);
    });

    const lastLine = createMemo((): string | undefined => {
        const chunks = props.node.log.chunks;
        if (!chunks.length) return undefined;
        // Walk from the end to find the last non-empty stdout/stderr line
        for (let i = chunks.length - 1; i >= 0; i--) {
            const c = chunks[i];
            if ((c.kind === "stdout" || c.kind === "stderr") && c.content.trim()) {
                return c.content.trim();
            }
        }
        return undefined;
    });

    const statusSigil = createMemo(() => {
        switch (props.node.status) {
            case "running": return "⟩";
            case "exited-ok": return "✓";
            case "exited-err": return "✗";
            case "stopped": return "■";
        }
    });

    const expanded = () => props.pinned;

    // Peek tooltip (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25).
    // Suppressed once expanded — the full command is already visible in the
    // panel header (`.agent-shell-header-cmd`), same rule ToolBlock uses.
    const peekTick = useTick(1000);
    const { isPeeking, rowEl: peekRowEl, setRowEl: setPeekRowEl, handlePeekEnter, handlePeekLeave } = useNodePeek();
    const peekTimeText = createMemo(() => {
        if (!isPeeking()) return null;
        peekTick();
        return `${formatExactTime(props.node.spawnedAt)} · ${formatTimeAgo(props.node.spawnedAt)}`;
    });
    const peekEstimateText = createMemo(() => {
        const count = estimateTokenCount(props.node.cmd);
        return count > 0 ? `~${formatCompactNumber(count)} tok (est.)` : null;
    });

    // Stop button (Phase 3): tree-kills the running process. Status updates
    // arrive via the `stopped` exit event, so we don't optimistically mutate.
    const [stopping, setStopping] = createSignal(false);
    const handleStop = async (e: MouseEvent) => {
        e.stopPropagation(); // don't toggle the expand panel
        if (stopping()) return;
        setStopping(true);
        try {
            await RpcApi.ShellStopCommand(TabRpcClient, { shell_id: props.node.id });
        } catch {
            // Best-effort — the exit event reconciles the final status.
        } finally {
            setStopping(false);
        }
    };

    // Collapse redraws first, over the raw append-only chunk stream, THEN cap
    // the (already deduplicated) result to the line budget — not the other
    // way around. Capping the raw stream first would let spinner/progress
    // noise (which is exactly what tends to dominate a long-running
    // command's raw chunk count) evict real content from the budget before
    // collapseSpinnerChunks ever got a chance to fold it down to one line.
    // It also matters for perf: capChunksByLines' windowed output slides its
    // start reference on every new chunk once a stream is sustained over
    // budget, which would defeat createSpinnerCollapser's append-only
    // identity tracking if fed the capped (rather than raw) chunks — reagent
    // P1, PR #2330 (the O(n·L²) full-window redraw rescan on every streamed
    // chunk this replaces).
    const spinnerCollapse = createSpinnerCollapser<ToolLogChunk>();
    const chunkCap = createChunkCapper(MAX_TOOL_OUTPUT_LINES);
    const cappedView = createMemo(() => {
        const { display: collapsed, spinnerSlot } = spinnerCollapse(props.node.log.chunks as ToolLogChunk[]);
        const { chunks: display, hiddenLines } = chunkCap(collapsed);
        return { display, spinnerSlot, hiddenLines };
    });
    const visibleChunks = () => cappedView().display;
    const hiddenCount = () => cappedView().hiddenLines;

    return (
        <div
            ref={setPeekRowEl}
            class={clsx("agent-shell-block", {
                "running": props.node.status === "running",
                "exited-ok": props.node.status === "exited-ok",
                "exited-err": props.node.status === "exited-err",
                "stopped": props.node.status === "stopped",
                "expanded": expanded(),
                "collapsed": !expanded(),
            })}
            onMouseEnter={handlePeekEnter}
            onMouseLeave={handlePeekLeave}
        >
            <div class="agent-shell-summary" onClick={props.onTogglePin}>
                <span class="agent-shell-sigil">{statusSigil()}</span>
                <span class="agent-shell-title">{props.node.title}</span>
                <span class="agent-shell-elapsed">[{elapsed()}]</span>
                <Show when={lastLine()}>
                    <span class="agent-shell-live-tail">↳ {lastLine()}</span>
                </Show>
                <Show when={props.node.status === "running"}>
                    <button
                        class="agent-shell-stop"
                        title="Stop process"
                        disabled={stopping()}
                        onClick={handleStop}
                    >
                        ■
                    </button>
                </Show>
            </div>
            <div
                class={clsx("agent-tool-panel", {
                    "agent-tool-panel--hidden": !expanded(),
                    "agent-tool-panel--flow": expanded(),
                })}
                inert={!expanded()}
                aria-hidden={!expanded()}
                onClick={(e) => e.stopPropagation()}
            >
                <div class="agent-tool-overlay">
                    <div class="agent-tool-overlay-header">
                        <span class="agent-shell-header-cmd">{props.node.cmd}</span>
                        <Show when={props.node.exitCode !== undefined}>
                            <span class="agent-tool-overlay-status-label">
                                exit {props.node.exitCode}
                            </span>
                        </Show>
                    </div>
                    <div class="agent-tool-overlay-log">
                        <Show when={hiddenCount() > 0}>
                            <OutputHiddenMarker hidden={hiddenCount()} noun="line" from="tail" />
                        </Show>
                        <For each={visibleChunks()}>
                            {(chunk) => (
                                <pre class={`agent-tool-log-line ${KIND_CLASS[chunk.kind] ?? ""}`}>
                                    <LinkifiedText text={capChars(chunk.content)} />
                                </pre>
                            )}
                        </For>
                        <Show when={cappedView().spinnerSlot !== null}>
                            <pre class={`agent-tool-log-line ${KIND_CLASS[cappedView().spinnerSlot?.kind ?? ""] ?? ""}`}>
                                {cappedView().spinnerSlot?.content}
                            </pre>
                        </Show>
                        <Show when={props.node.log.open}>
                            <div class="agent-shell-streaming-indicator" />
                        </Show>
                    </div>
                </div>
            </div>
            <PeekOverlay show={isPeeking() && !expanded()} rowEl={peekRowEl}>
                <Show when={peekTimeText()}>
                    <div class="agent-node-peek-tooltip-meta">{peekTimeText()}</div>
                </Show>
                <Show when={peekEstimateText()}>
                    <div class="agent-node-peek-tooltip-meta">{peekEstimateText()}</div>
                </Show>
                <div class="agent-node-peek-tooltip-body">{props.node.cmd}</div>
            </PeekOverlay>
        </div>
    );
};

PersistentShellBlock.displayName = "PersistentShellBlock";
