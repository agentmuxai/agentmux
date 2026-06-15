// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolBlock — single-line collapsed-by-default tool display with
 * click-to-pin and active-state auto-expand.
 *
 * Behavior (since SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28.md):
 *   - Collapsed (default): one line showing status icon + tool name +
 *     duration + (while streaming) the live-tail line. Applies to ALL
 *     terminated statuses.
 *   - Auto-expand: `running` and `pending_approval` keep the panel open
 *     in flow. After a terminal transition the panel stays open for
 *     POST_COMPLETION_HOLD_MS so the user can finish reading, then
 *     collapses.
 *   - Click summary: pins the expanded state. Clicking again unpins.
 *   - Hover: nothing happens. No browser-native tooltip, no panel
 *     expand, no time popup. The hover-to-peek model from the prior
 *     `tool-collapse.md` spec was removed — three overlapping visuals
 *     (browser title tooltip, larger log panel, fast expand/collapse)
 *     collapsed into a single auto-expand panel.
 *
 * SolidJS reactivity note:
 *   Props are accessed via `props.X` (never destructured in the function
 *   signature). Destructuring a SolidJS component's props captures the
 *   value at mount time and breaks reactivity for any prop that changes
 *   without triggering a parent re-render of the component. This bit us
 *   in an earlier version of this file: `pinned` was destructured, and
 *   pin toggles — which mutate `documentState` but not the document
 *   array — never reached the component, so the pin state appeared to
 *   reset on the next render cycle.
 */

import clsx from "clsx";
import { Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import type { BashResult, EditResult, GlobResult, GrepResult, WriteResult } from "../types";
import { createBlock } from "@/store/global";
import type { ToolNode } from "../types";
import { ToolBlockOverlay } from "./ToolBlockOverlay";

// Ticks every second while a tool is running with no output yet.
function ToolElapsedTicker(props: { startMs: number }): JSX.Element {
    const [elapsed, setElapsed] = createSignal(
        Math.floor((Date.now() - props.startMs) / 1000)
    );
    const interval = setInterval(
        () => setElapsed(Math.floor((Date.now() - props.startMs) / 1000)),
        1000
    );
    onCleanup(() => clearInterval(interval));
    return <>{elapsed()}s…</>;
}

interface ToolBlockProps {
    node: ToolNode;
    /** User has clicked to pin this tool block open. */
    pinned: boolean;
    /** Toggle the pinned state (called on click of the collapsed row). */
    onTogglePin: () => void;
    /** Opens the tool's overlay content in a dedicated pane. */
    onOpenInPane?: () => void;
}

const STATUS_ICON: Record<ToolNode["status"], string> = {
    running: "⏳",
    pending_approval: "⚠",
    awaiting_answer: "❓",
    success: "✓",
    failed: "✗",
    denied: "⊘",
    canceled: "⏹",
};

export const ToolBlock = (props: ToolBlockProps): JSX.Element => {
    // Stays true for POST_COMPLETION_HOLD_MS after a running tool
    // completes so the user can read the final output line before the
    // panel collapses.
    // - Originally 1s (#988).
    // - Bumped to 5s in #1006 — too tight to finish reading.
    // - Dropped to 3s 2026-05-26 — 5s felt too long during live
    //   conversation; user wants it punchier.
    const POST_COMPLETION_HOLD_MS = 3000;
    const [postCompletionHold, setPostCompletionHold] = createSignal(false);
    // Gate the post-completion hold on a real active → inactive
    // TRANSITION (not on a status-value snapshot). The earlier draft
    // simply checked `s !== "running" && ...` which fired on mount
    // for already-completed tools — loaded transcripts would briefly
    // auto-expand every completed tool row on initial render
    // (codex P1 round 2 on #988).
    //
    // Background on the older self-loop bug (round 1): reading
    // `postCompletionHold()` inside the same effect that wrote to it
    // made the effect a subscriber of its own write; the synchronous
    // re-run disposed the previous owner and ran the just-registered
    // `onCleanup(() => clearTimeout(t))` BEFORE the timer could fire,
    // leaving the panel auto-expanded forever after the first
    // completion. Both bugs are fixed here: track only
    // `props.node.status`, and gate on a transition by comparing
    // against `prevStatus` captured outside the reactive scope.
    //
    // prevNodeId guards against <Index> slot-position state leakage:
    // when a streaming-buffer cap-advance replaces the node at this
    // slot position with a different node, the old prevStatus must not
    // be treated as a transition baseline for the incoming node.
    let prevStatus: string = props.node.status;
    let prevNodeId: string = props.node.id;
    const isActive = (s: string): boolean =>
        s === "running" || s === "pending_approval";
    createEffect(() => {
        const s = props.node.status;
        const id = props.node.id;
        if (id !== prevNodeId) {
            prevNodeId = id;
            prevStatus = s;
            setPostCompletionHold(false);
            return;
        }
        if (isActive(prevStatus) && !isActive(s)) {
            setPostCompletionHold(true);
            const t = setTimeout(() => setPostCompletionHold(false), POST_COMPLETION_HOLD_MS);
            onCleanup(() => clearTimeout(t));
        }
        prevStatus = s;
    });

    // Auto-expand while the tool is actively running (or awaiting
    // approval). Terminal states (success OR failure) get the 5s
    // post-completion hold then collapse. Pin still wins as an
    // explicit override (so the user can keep a completed tool
    // expanded). Hover keeps working as a peek affordance for
    // collapsed (completed) tools — successes and failures alike.
    //
    // Per SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md §4.2 — Phase B.
    // The 2026-05-24 user feedback removed `failed` from the
    // always-expanded set; failed-collapses-after-5s mirrors the
    // success path, and the ✗ icon + red border-left at the
    // collapsed row continue to flag the failure.
    const autoExpanded = (): boolean => {
        const s = props.node.status;
        return s === "running" || s === "pending_approval"
            || postCompletionHold();
    };
    // Hover-to-peek was removed in SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28
    // — expansion is now driven exclusively by pin + active-state auto-
    // expand. The user-visible "three popups on hover" (browser title
    // tooltip + larger log panel + fast expand/collapse animation)
    // collapsed into a single in-flow panel.
    //
    // Exception: if the user's mouse is inside an already-expanded block,
    // we hold it open until they leave so the post-completion timer can't
    // collapse it mid-read.
    const [userHolding, setUserHolding] = createSignal(false);
    const expanded = () => props.pinned || autoExpanded() || userHolding();

    // Result pill — compact inline summary shown at medium+ pane widths
    // (visible only via CSS container query; always rendered so the
    // DOM is stable when the pane is resized through the breakpoint).
    const resultPill = (): { label: string; variant: string } | null => {
        const s = props.node.status;
        if (s === "running" || s === "pending_approval" || !props.node.result) return null;
        const r = props.node.result as any;
        switch (props.node.tool) {
            case "Bash": {
                const br = r as BashResult;
                let code = br.exitCode;
                // Claude provider encodes exit code in stdout as "<exited N>" (claude-translator.ts:263)
                // rather than populating exitCode on the result object.
                if (typeof code !== "number" && typeof br.stdout === "string") {
                    const m = br.stdout.match(/<exited\s+(\d+)>/);
                    if (m) code = parseInt(m[1], 10);
                }
                if (typeof code === "number") {
                    return code === 0
                        ? { label: "exit 0", variant: "exit-ok" }
                        : { label: `exit ${code}`, variant: "exit-err" };
                }
                return null;
            }
            case "Glob": {
                const n = (r as GlobResult).files?.length;
                if (typeof n === "number") {
                    return { label: `${n} file${n === 1 ? "" : "s"}`, variant: "files" };
                }
                return null;
            }
            case "Grep": {
                const n = (r as GrepResult).matches?.length;
                if (typeof n === "number") {
                    return { label: `${n} match${n === 1 ? "" : "es"}`, variant: "matches" };
                }
                return null;
            }
            case "Write": {
                const b = (r as WriteResult).bytesWritten;
                return typeof b === "number"
                    ? { label: `${b}b`, variant: "written" }
                    : { label: "written", variant: "written" };
            }
            case "Edit": {
                const n = (r as EditResult).linesChanged;
                return typeof n === "number"
                    ? { label: `${n} line${n === 1 ? "" : "s"}`, variant: "edited" }
                    : { label: "edited", variant: "edited" };
            }
            case "Agent":
                return s === "success" ? { label: "done", variant: "agent" } : null;
            default:
                return null;
        }
    };

    // Two render modes — `flow` when the panel is visible (auto-expand
    // or pinned), `hidden` otherwise. The hover-only `overlay` mode is
    // gone with the hover trigger.
    const panelMode = (): "hidden" | "flow" => (expanded() ? "flow" : "hidden");

    const statusIcon = (): string => STATUS_ICON[props.node.status] || "•";

    return (
        <div
            class={clsx("agent-tool-block", {
                collapsed: !expanded(),
                expanded: expanded(),
                pinned: props.pinned,
                running: props.node.status === "running",
                success: props.node.status === "success",
                failed: props.node.status === "failed",
                canceled: props.node.status === "canceled",
                pending_approval: props.node.status === "pending_approval",
                awaiting_answer: props.node.status === "awaiting_answer",
                denied: props.node.status === "denied",
            })}
            data-tool={props.node.tool.toLowerCase()}
            onMouseEnter={() => { if (props.pinned || autoExpanded()) setUserHolding(true); }}
            onMouseLeave={() => setUserHolding(false)}
        >
            <div class="agent-tool-summary" onClick={props.onTogglePin}>
                <span class="agent-tool-status-icon">{statusIcon()}</span>
                <span class="agent-tool-name">{props.node.summary}</span>
                <Show when={props.node.duration}>
                    <span class="agent-tool-duration">({props.node.duration.toFixed(1)}s)</span>
                </Show>
                <Show when={resultPill() != null}>
                    <span class={`agent-tool-result-pill pill-${resultPill()?.variant}`}>
                        {resultPill()?.label}
                    </span>
                </Show>
                {/* Live-tail: while streaming, show the last stdout/stderr
                    line so the user can watch real output without opening
                    the overlay. Skips kind:"system" chunks — those are
                    bashwrap internals ("[bashwrap] starting: N chars", PTY
                    ready, etc.) and are not useful here. If no stdout/stderr
                    has arrived yet (e.g. during a `sleep` prefix), show an
                    elapsed timer instead so the user knows it's alive. */}
                <Show when={props.node.log?.open === true}>
                    {(() => {
                        const chunks = props.node.log?.chunks ?? [];
                        // Walk backwards — find the last real output chunk
                        let lastOutput: { kind: string; content: string } | undefined;
                        for (let i = chunks.length - 1; i >= 0; i--) {
                            const c = chunks[i];
                            if (c.kind === "stdout" || c.kind === "stderr") {
                                lastOutput = c;
                                break;
                            }
                        }
                        if (lastOutput) {
                            return (
                                <span
                                    class="agent-tool-live-tail"
                                    title={`latest stream output (${chunks.length} chunks)`}
                                >
                                    ↳ {lastOutput.content}
                                </span>
                            );
                        }
                        // No stdout/stderr yet — show elapsed time
                        return (
                            <span class="agent-tool-live-tail agent-tool-live-tail--waiting">
                                <ToolElapsedTicker startMs={props.node.timestamp ?? Date.now()} />
                            </span>
                        );
                    })()}
                </Show>
                <Show when={props.node.tool === "Agent"}>
                    <button
                        class="agent-tool-open-pane"
                        title="Open subagent in new pane"
                        onClick={(e) => {
                            e.stopPropagation();
                            const agentId = (props.node.params as any).subagent_id || props.node.id;
                            createBlock({
                                meta: {
                                    view: "subagent",
                                    "subagent:id": agentId,
                                } as any,
                            });
                        }}
                    >
                        ⧉
                    </button>
                </Show>
            </div>
            {/* Panel — three render modes per `panelMode()`:
             *
             *   hidden  → `.agent-tool-panel--hidden` (off).
             *   flow    → in-flow under the summary (default DOM
             *             layout — pinned / running / post-hold).
             *   overlay → absolute positioning above OR below the
             *             summary so a hover near the pane's bottom
             *             expands upward instead of being clipped.
             *
             * Always rendered in the DOM so CSS transitions can
             * animate the off→on shift; `inert` + `aria-hidden`
             * remove it from the focus/a11y tree when hidden.
             */}
            <div
                class={clsx("agent-tool-panel", {
                    "agent-tool-panel--hidden": panelMode() === "hidden",
                    "agent-tool-panel--flow": panelMode() === "flow",
                })}
                // Codex P2 on #988: with the always-rendered markup, the
                // collapsed panel was visually hidden via max-height /
                // opacity but still in the focusable + a11y tree, so
                // keyboard users could tab into action buttons that
                // aren't visible. `inert` removes the entire subtree
                // from focus + accessibility while collapsed (Chrome 102+,
                // supported in the bundled CEF runtime).
                inert={!expanded()}
                aria-hidden={!expanded()}
                onClick={(e) => e.stopPropagation()}
            >
                <ToolBlockOverlay
                    node={props.node}
                    onOpenInPane={props.onOpenInPane}
                />
            </div>
        </div>
    );
};

ToolBlock.displayName = "ToolBlock";
