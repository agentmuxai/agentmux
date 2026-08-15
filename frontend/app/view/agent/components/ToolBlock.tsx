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
 *     in flow. After a terminal transition the panel stays open while the
 *     tool is held (`props.heldOpen`, backed by `documentState.expandedTools`)
 *     — i.e. while it's still on screen — and collapses once its row scrolls
 *     off the top. This replaced the old fixed post-completion timer; see
 *     docs/specs/PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md.
 *   - Click summary: pins the expanded state. Clicking again unpins.
 *   - Hover (collapsed only): no panel expand, no time popup — the
 *     hover-to-peek model from the prior `tool-collapse.md` spec is still
 *     gone (that removed three overlapping visuals: browser title tooltip,
 *     larger log panel, fast expand/collapse — collapsed into the single
 *     auto-expand panel above). What DOES still show on hover is a small,
 *     separate tooltip over just the command/summary text — the full
 *     word-wrapped string, nothing else (no output, no expand trigger).
 *     Suppressed once the panel is already expanded, since the command is
 *     visible in context there. This is intentionally narrower than what
 *     was removed: static text only, no state change.
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
import { Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatExactTime, formatTimeAgo } from "@/util/format-time";
import type { BashResult, EditResult, GlobResult, GrepResult, WriteResult } from "../types";
import type { ToolNode } from "../types";
import { ToolBlockOverlay } from "./ToolBlockOverlay";
import { extractToolDetail } from "../stream-parser";
import { PeekOverlay } from "./PeekOverlay";

/**
 * Ref callback that plays a one-shot fade-in animation ONLY on a genuine
 * Solid-level mount of the element (i.e. when the enclosing `<Show>`
 * flips true because the tool's status actually changed). A persistent
 * CSS `animation:` on the selector itself would ALSO retrigger on any
 * unrelated `display:none` <-> visible toggle — e.g. `.agent-tool-result-
 * pill`'s `@container` breakpoint at 600px (`_responsive.scss`) applies
 * regardless of tool status, so resizing the pane across that width would
 * replay the fade-in on every already-completed, unrelated tool call
 * (reagent P2 on PR #1975). A ref only fires once, at real DOM insertion.
 *
 * The `classList.add` is deferred a microtask: `ref` callbacks fire at
 * element CREATION, before Solid's own effect for this element's dynamic
 * `class={...}` expression has run (that effect sets `el.className`
 * wholesale, not via `classList`) — mutating classList synchronously in
 * the ref gets immediately clobbered by that later assignment. Deferring
 * past the current synchronous render lets Solid's class effect finish
 * first, so this addition survives.
 */
function fadeInOnMount(el: HTMLElement): void {
    queueMicrotask(() => {
        el.classList.add("agent-tool-fade-in-once");
        el.addEventListener("animationend", () => el.classList.remove("agent-tool-fade-in-once"), {
            once: true,
        });
    });
}

function ToolElapsedTicker(props: { startMs: number }): JSX.Element {
    const tick = useTick(1000);
    const elapsed = createMemo(() => (tick(), Math.floor((Date.now() - props.startMs) / 1000)));
    return <>{elapsed()}s…</>;
}

interface ToolBlockProps {
    node: ToolNode;
    /** User has clicked to pin this tool block open. */
    pinned: boolean;
    /**
     * Held expanded after completing live on screen (`documentState.expandedTools`).
     * Set by `onHoldOpen` on completion; cleared by the pane's scroll-off scan
     * once the row leaves the top — the scroll-driven replacement for the old
     * 3 s post-completion timer.
     */
    heldOpen?: boolean;
    /** Toggle the pinned state (called on click of the collapsed row). */
    onTogglePin: () => void;
    /** Mark this tool held-open — called once on its active→inactive transition. */
    onHoldOpen?: () => void;
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

const PREVIEW_ZOOM_STEP = 0.05;
const PREVIEW_ZOOM_MIN = 0.7;
const PREVIEW_ZOOM_MAX = 2.0;

// 150ms enter-delay matches UserMessageBlock's hover-to-peek — prevents
// accidental expansions during fast scroll-throughs.
const PEEK_ENTER_DELAY_MS = 150;

export const ToolBlock = (props: ToolBlockProps): JSX.Element => {
    // Drives the peek tooltip's live "time ago" text (§2.3 of
    // SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md). Unconditional, same
    // low-cost precedent as ToolElapsedTicker above — not gated on whether
    // the tooltip is actually open, since Tooltip doesn't expose that state
    // to its caller.
    const peekTick = useTick(1000);

    // Independent font-scale for the tool preview panel. Ctrl+Scroll inside the
    // panel zooms only the preview; the pane-level zoom (block.meta["term:zoom"])
    // is unaffected. Ephemeral — not persisted to block meta.
    const [previewFontScale, setPreviewFontScale] = createSignal(1.0);
    let panelRef: HTMLDivElement | undefined;
    onMount(() => {
        if (!panelRef) return;
        const onWheel = (e: WheelEvent) => {
            if (!e.ctrlKey) return;
            // Only zoom the PREVIEW when the pointer is over the preview body.
            // Over the file-path header (or anywhere else in the panel), let
            // Ctrl+wheel bubble through to the pane zoom (term:zoom) — the header
            // belongs to the pane, not the preview. Without this gate, Ctrl+wheel
            // over the filename zoomed the preview, and there was no way to zoom
            // the whole pane while hovering the tool block.
            const t = e.target as HTMLElement | null;
            const overPreviewBody =
                !!t?.closest(".agent-tool-overlay-log") &&
                !t.closest(".agent-tool-file-path, .agent-tool-file-path-row");
            if (!overPreviewBody) return; // fall through → pane zoom
            e.preventDefault();
            e.stopPropagation();
            const delta = e.deltaY < 0 ? PREVIEW_ZOOM_STEP : -PREVIEW_ZOOM_STEP;
            setPreviewFontScale(prev =>
                Math.min(PREVIEW_ZOOM_MAX, Math.max(PREVIEW_ZOOM_MIN, prev + delta))
            );
        };
        panelRef.addEventListener("wheel", onWheel, { passive: false });
        onCleanup(() => panelRef?.removeEventListener("wheel", onWheel));
    });

    // A tool stays expanded after completing live until its row scrolls off the
    // top — the "post-completion hold" now lives in `documentState.expandedTools`
    // (read via props.heldOpen) instead of a 3 s timer. Here we just detect the
    // active → inactive TRANSITION and mark the tool held-open via onHoldOpen.
    //
    // Gate on a real TRANSITION (not a status-value snapshot): firing on mount
    // for an already-completed tool would auto-expand every row of a loaded
    // transcript (codex P1 round 2 on #988). prevNodeId guards <Index>
    // slot-position state leakage — when a streaming-buffer cap-advance swaps the
    // node at this slot, the old prevStatus must not seed the incoming node's
    // transition baseline.
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
            return;
        }
        if (isActive(prevStatus) && !isActive(s)) {
            props.onHoldOpen?.();
        }
        prevStatus = s;
    });

    // Auto-expand while the tool is actively running (or awaiting approval), and
    // keep a completed tool expanded while it's held open (props.heldOpen — set
    // on completion, cleared when the row scrolls off the top). Pin still wins as
    // an explicit override. `denied`/`canceled` are user-dismissed terminations
    // (the user already acted on them) and skip the heldOpen hold, collapsing
    // immediately. `failed` is an agent-caused error, not a dismissal — it holds
    // open exactly like `success` per the original design doc's own
    // recommendation (ANALYSIS_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md,
    // "failed: same as success"); the earlier implementation had lumped it in
    // with denied/canceled, diverging from that call.
    //
    // Per SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md §4.2 and
    // docs/specs/PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md — the 3 s
    // post-completion timer was replaced by scroll-position-driven collapse.
    const isFailTerminal = (): boolean => {
        const s = props.node.status;
        return s === "denied" || s === "canceled";
    };
    const autoExpanded = (): boolean => {
        const s = props.node.status;
        return s === "running" || s === "pending_approval"
            || (!isFailTerminal() && !!props.heldOpen);
    };
    // Hover-to-peek was removed in SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28
    // — expansion is now driven exclusively by pin + active-state auto-
    // expand. The user-visible "three popups on hover" (browser title
    // tooltip + larger log panel + fast expand/collapse animation)
    // collapsed into a single in-flow panel.
    //
    // Exception: if the user's mouse is inside an already-expanded block,
    // we hold it open until they leave so a scroll-off collapse can't fold it
    // mid-read.
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

    // Bare command/detail text for the hover tooltip — same per-tool-kind
    // extraction generateToolSummary() uses for the decorated `summary`
    // string, so the two never drift out of sync. Suppressed once the
    // panel is already expanded (command visible in context there) or
    // when there's nothing tool-kind-specific to show.
    const cmdText = createMemo(() => extractToolDetail(props.node.tool, (props.node.params as Record<string, any>) ?? {}));

    // Peek-tooltip time + estimate lines (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md
    // §2.3). Real API-reported token/cost data doesn't exist per-tool-call
    // today (see SPEC_PER_NODE_TOKEN_ACCOUNTING_2026_08_03.md for the real,
    // Claude-only derivation planned as a follow-up) — this is a client-side
    // chars÷4 estimate, always labeled "(est.)".
    //
    // isPeeking tracks REAL hover state over the tooltip anchor (set by the
    // wrapping span below). reagent P2 on PR #2392: peekTimeText used to
    // read peekTick() unconditionally, so every mounted ToolBlock — every
    // completed tool row in a whole transcript — subscribed to the shared 1s
    // ticker forever, not just the one actually being hovered right now.
    // Short-circuiting before peekTick() means only genuinely-hovered rows
    // subscribe.
    const [isPeeking, setIsPeeking] = createSignal(false);
    const peekTimeText = createMemo(() => {
        if (!isPeeking()) return null;
        peekTick(); // re-run every second so "ago" stays live while hovered
        const ts = props.node.timestamp;
        if (ts == null) return null;
        return `${formatExactTime(ts)} · ${formatTimeAgo(ts)}`;
    });
    const peekEstimateText = createMemo(() => {
        const text = JSON.stringify(props.node.params ?? {}) + JSON.stringify(props.node.result ?? "");
        const count = estimateTokenCount(text);
        return count > 0 ? `~${formatCompactNumber(count)} tok (est.)` : null;
    });
    // Raw-data existence check for showing the overlay at all — deliberately
    // NOT reading peekTimeText()/peekEstimateText() themselves, since
    // peekTimeText is gated behind isPeeking() (only resolves to a real
    // value once hover already started). Using the ticking memos here would
    // make the show-check circularly depend on hover having already begun,
    // defeating its own purpose of deciding whether hover should open
    // anything in the first place.
    const hasAnyPeekContent = createMemo(() =>
        cmdText() !== "" || props.node.timestamp != null || peekEstimateText() != null
    );

    // Peek overlay — Portal-rendered (see PeekOverlay.tsx's doc comment for
    // why: each virtualized row is its own CSS stacking context, so a plain
    // `position: absolute` child can never paint above a LATER row no
    // matter its z-index — confirmed live via CDP, the next row's own
    // content painted over it). Styled and positioned like
    // UserMessageBlock.tsx's "Session context" hover-to-peek (flush to the
    // row's edges, no gap), reusing the same `hover-anchor.ts` direction
    // logic — just rendered outside the row's subtree instead of inside it.
    let peekEnterTimer: ReturnType<typeof setTimeout> | undefined;
    let rowEl: HTMLDivElement | undefined;

    const handlePeekEnter = () => {
        clearTimeout(peekEnterTimer);
        peekEnterTimer = setTimeout(() => setIsPeeking(true), PEEK_ENTER_DELAY_MS);
    };
    const handlePeekLeave = () => {
        clearTimeout(peekEnterTimer);
        setIsPeeking(false);
    };
    onCleanup(() => clearTimeout(peekEnterTimer));

    return (
        <div
            ref={(el) => (rowEl = el)}
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
            // Raw provider tool name, distinct from `data-tool` above: `tool` is normalized to
            // a coarse closed set (unrecognized names, e.g. "AskUserQuestion", collapse to
            // "other" — see normalizeToolName in stream-parser.ts), which loses exactly the
            // names CSS needs to target one specific open-ended tool without touching the
            // shared "other" styling every other unrecognized tool also falls back to.
            data-tool-name={props.node.toolName?.toLowerCase()}
            onMouseEnter={() => { if (props.pinned || autoExpanded()) setUserHolding(true); }}
            onMouseLeave={() => setUserHolding(false)}
        >
            <div class="agent-tool-summary" onClick={props.onTogglePin}>
                <span class="agent-tool-status-icon">{statusIcon()}</span>
                <span
                    class="agent-tool-name-peek-anchor"
                    onMouseEnter={handlePeekEnter}
                    onMouseLeave={handlePeekLeave}
                >
                    <span class="agent-tool-name">{props.node.summary}</span>
                </span>
                <Show when={props.node.duration}>
                    <span class="agent-tool-duration">({props.node.duration.toFixed(1)}s)</span>
                </Show>
                <Show when={resultPill() != null}>
                    <span
                        ref={fadeInOnMount}
                        class={`agent-tool-result-pill pill-${resultPill()?.variant}`}
                    >
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
            </div>
            {/* Peek overlay — SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md,
                styled to match UserMessageBlock.tsx's "Session context"
                hover-to-peek (see PeekOverlay.tsx). Suppressed once the
                panel is already expanded, since the command/time are
                visible in context there — same condition the old
                Tooltip-based version used. */}
            <PeekOverlay
                show={isPeeking() && hasAnyPeekContent() && !expanded()}
                rowEl={() => rowEl}
            >
                <Show when={peekTimeText()}>
                    <div class="agent-node-peek-tooltip-meta">{peekTimeText()}</div>
                </Show>
                <Show when={peekEstimateText()}>
                    <div class="agent-node-peek-tooltip-meta">{peekEstimateText()}</div>
                </Show>
                <Show when={cmdText()}>
                    <div class="agent-node-peek-tooltip-body">{cmdText()}</div>
                </Show>
            </PeekOverlay>
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
                ref={panelRef}
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
                    previewFontScale={previewFontScale}
                />
            </div>
        </div>
    );
};

ToolBlock.displayName = "ToolBlock";
