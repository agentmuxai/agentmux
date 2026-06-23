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
import { For, Show, createEffect, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import { capChars, createChunkCapper, MAX_TOOL_OUTPUT_LINES } from "./output-cap";

const SPINNER_CHARS = new Set([
    '⠋','⠙','⠹','⠸','⠼','⠴','⠦','⠧','⠇','⠏',
    '⣾','⣽','⣻','⢿','⡿','⣟','⣯','⣷',
    '◐','◓','◑','◒','◴','◷','◶','◵',
    '-','\\','|','/',
]);
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { LinkifiedText } from "@/app/element/linkified-text";
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

function formatElapsed(ms: number): string {
    const totalSec = Math.floor(ms / 1000);
    const m = Math.floor(totalSec / 60);
    const s = totalSec % 60;
    return `${m}:${String(s).padStart(2, "0")}`;
}

export const PersistentShellBlock = (props: PersistentShellBlockProps): JSX.Element => {
    const [now, setNow] = createSignal(Date.now());

    // Gate the interval on the status memo, not props.node, so chunk appends
    // don't tear down and recreate the timer on every streamed line.
    const isRunning = createMemo(() => props.node.status === "running");
    createEffect(() => {
        if (!isRunning()) return;
        const id = setInterval(() => setNow(Date.now()), 1000);
        onCleanup(() => clearInterval(id));
    });

    const elapsed = createMemo(() => {
        const end = props.node.exitedAt ?? now();
        return formatElapsed(end - props.node.spawnedAt);
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

    // createChunkCapper tracks total lines incrementally and returns hiddenLines.
    // Using capChunksByLines directly only returns keptLines (no hidden count).
    const chunkCap = createChunkCapper(MAX_TOOL_OUTPUT_LINES);
    const cappedView = createMemo(() => {
        const { chunks, hiddenLines } = chunkCap(props.node.log.chunks as ToolLogChunk[]);
        // Collapse consecutive spinner-char runs: trailing run → live slot,
        // completed run → frozen last frame. Mirrors ToolOverlayLog ChunkList.
        const display: ToolLogChunk[] = [];
        let spinnerSlot: { content: string; kind: string } | null = null;
        for (let i = 0; i < chunks.length; i++) {
            const chunk = chunks[i];
            const trimmed = capChars(chunk.content).trim();
            if (SPINNER_CHARS.has(trimmed)) {
                let last = chunk;
                while (i + 1 < chunks.length && SPINNER_CHARS.has(capChars(chunks[i + 1].content).trim())) {
                    i++;
                    last = chunks[i];
                }
                const lastFrame = capChars(last.content).trim();
                if (i === chunks.length - 1) {
                    spinnerSlot = { content: lastFrame, kind: last.kind };
                } else {
                    display.push({ ...last, content: lastFrame });
                    spinnerSlot = null;
                }
            } else {
                display.push(chunk);
                spinnerSlot = null;
            }
        }
        return { display, spinnerSlot, hiddenLines };
    });
    const visibleChunks = () => cappedView().display;
    const hiddenCount = () => cappedView().hiddenLines;

    return (
        <div
            class={clsx("agent-shell-block", {
                "running": props.node.status === "running",
                "exited-ok": props.node.status === "exited-ok",
                "exited-err": props.node.status === "exited-err",
                "stopped": props.node.status === "stopped",
                "expanded": expanded(),
                "collapsed": !expanded(),
            })}
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
        </div>
    );
};

PersistentShellBlock.displayName = "PersistentShellBlock";
