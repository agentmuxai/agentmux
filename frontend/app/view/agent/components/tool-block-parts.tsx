// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The parts of a ToolBlock row: the summary line (status, name, note, duration, result pill,
 * live tail), the peek popover, and the panel with its actions and body. ToolBlock owns the state
 * (tool-block-state.ts) and hands it down; nothing here keeps state of its own beyond the panel's
 * element. Props are read as `props.X`, never destructured.
 */

import { IconButton } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { useTick } from "@/app/hook/useTick";
import clsx from "clsx";
import { Show, createMemo, onCleanup, type Accessor, type JSX } from "solid-js";
import { OpenInEditorButton } from "../agent-open-in-editor";
import { takeBodyHeight } from "../body-height-cache";
import type { AgentDispatch } from "../../swarm/swarm-model";
import type { ToolNode } from "../../../agent-model/types";
import { TOOL_STATUS } from "../../../agent-model/tool-status";
import { mcpDisplayName, toolNameOf } from "../../../agent-model/tool-descriptors";
import { PeekOverlay } from "./PeekOverlay";
import { PeekMetaRow } from "./PeekMetaRow";
import { ShellTokens } from "./shell-highlight/ShellCommand";
import { ToolBlockOverlay } from "./ToolBlockOverlay";
import { hasAuthoredSummary, type ToolHeaderParts } from "./tool-header";
import { markdownPreviewPath } from "./tool-renderers/builtins";
import { toolResultPill } from "./tool-result-pill";
import type { ToolDisclosure, ToolPeek, UnloadedResult } from "./tool-block-state";

/**
 * Ref callback that plays a one-shot fade-in animation ONLY on a genuine Solid-level mount of the
 * element (i.e. when the enclosing `<Show>` flips true because the tool's status actually
 * changed). A persistent CSS `animation:` on the selector itself would ALSO retrigger on any
 * unrelated `display:none` <-> visible toggle — e.g. `.agent-tool-result-pill`'s `@container`
 * breakpoint at 600px (`_responsive.scss`) applies regardless of tool status, so resizing the pane
 * across that width would replay the fade-in on every already-completed, unrelated tool call
 * (#1975). A ref only fires once, at real DOM insertion.
 *
 * The `classList.add` is deferred a microtask: `ref` callbacks fire at element CREATION, before
 * Solid's own effect for this element's dynamic `class={...}` expression has run (that effect sets
 * `el.className` wholesale, not via `classList`) — mutating classList synchronously in the ref
 * gets immediately clobbered by that later assignment. Deferring past the current synchronous
 * render lets Solid's class effect finish first, so this addition survives.
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
    // `startMs` is wall-clock, so a backwards system-clock step (NTP correction, manual set, VM
    // resume) mid-tool-call makes this negative — and this ticker exists specifically for calls
    // long-running enough to need one, which is exactly when a step has time to land. Rendered
    // raw it would print e.g. "-1717984694s…" in the transcript and Activity Dock, the same class
    // of bug as #2831 (status-bar uptime) and #2832 (sysinfo chart). `formatElapsedCompact` and
    // `formatElapsedClock` in `util/format-time` already clamp for the same reason. Clamping
    // rather than hiding: a stalled-looking "0s…" is honest about having lost the reference
    // point, where a negative is just wrong.
    const elapsed = createMemo(() => (tick(), Math.max(0, Math.floor((Date.now() - props.startMs) / 1000))));
    return <>{elapsed()}s…</>;
}

export const isBashTool = (node: ToolNode): boolean => node.tool.toLowerCase() === "bash";

/**
 * Live-tail: while streaming, the last stdout/stderr line, so the user can watch real output
 * without opening the overlay. Skips kind:"system" chunks — those are bashwrap internals
 * ("[bashwrap] starting: N chars", PTY ready, etc.) and are not useful here. If no stdout/stderr
 * has arrived yet (e.g. during a `sleep` prefix), an elapsed timer shows that it is alive. A plain
 * function, not a component: its reads of the log must stay tracked by the enclosing `<Show>`.
 */
function liveTail(node: ToolNode): JSX.Element {
    const chunks = node.log?.chunks ?? [];
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
            <span class="agent-tool-live-tail" {...tip(`latest stream output (${chunks.length} chunks)`)}>
                {/* U+2192 (→), not U+21B3 (↳) — see
                    SPEC_ACTIVITY_DOCK_TITLE_WIDTH_AND_TAIL_GLYPH_2026_09_05.md §3.1. */}
                → {lastOutput.content}
            </span>
        );
    }
    // No stdout/stderr yet — show elapsed time
    return (
        <span class="agent-tool-live-tail agent-tool-live-tail--waiting">
            <ToolElapsedTicker startMs={node.timestamp ?? Date.now()} />
        </span>
    );
}

/** The tool's name in the row: its icon, label, line range and argument, or an authored summary. */
function ToolNameText(props: { node: ToolNode; header: Accessor<ToolHeaderParts> }): JSX.Element {
    const header = props.header;
    const authoredSummary = () => hasAuthoredSummary(props.node);
    // A path or a command is cut from the left in the row, keeping its end in view.
    const cutFromStart = () => header().detailIsPath || (isBashTool(props.node) && header().detail !== "");
    return (
        <span class="agent-tool-name-peek-anchor">
            <Show when={!authoredSummary()} fallback={<span class="agent-tool-name">{props.node.summary}</span>}>
                {/* One text run (no whitespace between spans) so the row's textContent reads "🌐 solid docs". */}
                <span class="agent-tool-name" classList={{ "agent-tool-name--cut-start": cutFromStart() }}>
                    <span class="agent-tool-icon">{header().icon}</span>
                    <Show when={header().label}>
                        {" "}
                        <span class="agent-tool-label">{header().label}</span>
                    </Show>
                    {/* A Read's lines come before the path: the path is the part
                        that gets ellipsized, and the range is what you scan for. */}
                    <Show when={!authoredSummary() && header().range}>
                        {" "}
                        <span class="agent-tool-range">{header().range}</span>
                    </Show>
                    <Show when={header().detail}>
                        {" "}
                        <Show
                            when={isBashTool(props.node)}
                            fallback={
                                <Show
                                    when={header().detailIsPath}
                                    fallback={<span class="agent-tool-detail">{header().detail}</span>}
                                >
                                    {/* Cut from the left so the file name stays: the span runs
                                        right-to-left for the ellipsis, the <bdi> keeps the path
                                        itself reading left to right. */}
                                    <span class="agent-tool-detail agent-tool-detail--cut-start">
                                        <bdi>{header().detail}</bdi>
                                    </span>
                                </Show>
                            }
                        >
                            {/* A command is cut from the left too, so its end (the
                                part that differs between calls) stays in view. */}
                            <span class="agent-tool-detail agent-shell-inline agent-tool-detail--cut-start">
                                <bdi>
                                    <ShellTokens command={header().detail} />
                                </bdi>
                            </span>
                        </Show>
                    </Show>
                </span>
            </Show>
        </span>
    );
}

interface ToolSummaryProps {
    node: ToolNode;
    header: Accessor<ToolHeaderParts>;
    dispatchMatch?: AgentDispatch;
    peek: ToolPeek;
    view: ToolDisclosure;
}

/**
 * The line: status icon, name, note, duration, result pill, live tail. The peek belongs to the
 * tool call's line, anywhere on it, but not to its expanded output: a peek over the output
 * covered the text being read or selected
 * (SPEC_AGENT_PANE_TOOL_PEEK_ON_LINE_AND_MD_OPEN_IN_EDITOR_2026_10_11.md §1). The anchor moves
 * with the handlers, so the panel is placed against, and bridged from, the line.
 */
export function ToolSummary(props: ToolSummaryProps): JSX.Element {
    const statusIcon = (): string => TOOL_STATUS[props.node.status]?.icon || "•";
    const resultPill = () => toolResultPill(props.node, props.dispatchMatch);
    return (
        <div
            ref={props.peek.node.setRowEl}
            class="agent-tool-summary"
            onClick={props.view.onHeaderClick}
            onMouseEnter={props.peek.node.handlePeekEnter}
            onMouseLeave={props.peek.node.handlePeekLeave}
        >
            <span class="agent-tool-status-icon">{statusIcon()}</span>
            <ToolNameText node={props.node} header={props.header} />
            {/* Composed headers only: an authored summary already
                carries the note (the muxspect force-cancel writes
                both, for any tool — AskUserQuestion included). */}
            <Show when={!hasAuthoredSummary(props.node) && props.node.statusNote}>
                <span class="agent-tool-status-note">{props.node.statusNote}</span>
            </Show>
            <Show when={props.node.duration}>
                <span class="agent-tool-duration">({props.node.duration.toFixed(1)}s)</span>
            </Show>
            <Show when={resultPill() != null}>
                <span ref={fadeInOnMount} class={`agent-tool-result-pill pill-${resultPill()?.variant}`}>
                    {resultPill()?.label}
                </span>
            </Show>
            <Show when={props.node.log?.open === true}>{liveTail(props.node)}</Show>
        </div>
    );
}

/**
 * The peek overlay — SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md, see PeekOverlay.tsx. Every
 * line shows regardless of expand state: time and estimate since
 * REPORT_TOOL_CALL_PEEK_SUPPRESSED_WHEN_EXPANDED_2026_09_04.md, and the command line since
 * SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §1: the expanded panel doesn't
 * reliably show the full call (Bash's only appears once the result lands; most tools never show
 * all their params), and the header stays truncated. It is Portal-rendered (see PeekOverlay.tsx:
 * each virtualized row is its own CSS stacking context, so a plain `position: absolute` child can
 * never paint above a LATER row).
 */
export function ToolPeekOverlay(props: { node: ToolNode; header: Accessor<ToolHeaderParts>; peek: ToolPeek }): JSX.Element {
    const cmdText = () => props.header().detail;
    // The tool's name in front of its argument ("Bash ls", "Read /a/b.ts"): the row shows an icon
    // instead, so the popover names it. MCP tools read "server · tool", not their raw
    // mcp__server__tool name (SPEC_PEEK_PANEL_META_ROW_AND_MONO_COMMAND_2026_09_27.md §4.3).
    const peekToolName = () => {
        const name = toolNameOf(props.node);
        return mcpDisplayName(name) ?? name;
    };
    return (
        <PeekOverlay show={props.peek.node.isPeeking() && props.peek.hasAnyContent()} rowEl={props.peek.node.rowEl}>
            <PeekMetaRow time={props.peek.timeText()} tokens={props.peek.estimateText()} />
            <Show when={cmdText()}>
                <div class="agent-node-peek-tooltip-body" data-tool={props.node.tool.toLowerCase()}>
                    <span class="agent-node-peek-tooltip-tool">{peekToolName()}</span>{" "}
                    <Show when={isBashTool(props.node)} fallback={cmdText()}>
                        <span class="agent-shell-command">
                            <ShellTokens command={cmdText()} embedded />
                        </span>
                    </Show>
                </div>
            </Show>
        </PeekOverlay>
    );
}

interface ToolPanelProps {
    node: ToolNode;
    dispatchMatch?: AgentDispatch;
    view: ToolDisclosure;
    unloaded: UnloadedResult;
}

/** The preview's top-right actions: Open in Editor for a Markdown file, and the height toggle. */
function ToolPanelActions(props: { node: ToolNode; view: ToolDisclosure }): JSX.Element {
    return (
        <Show when={markdownPreviewPath(props.node) || props.view.showHeightToggle()}>
            <div class="agent-tool-panel-actions">
                <Show when={markdownPreviewPath(props.node)}>{(path) => <OpenInEditorButton path={path()} />}</Show>
                <Show when={props.view.showHeightToggle()}>
                    <IconButton
                        class="agent-tool-panel-height"
                        icon={props.view.tall() ? "compress" : "expand"}
                        label={props.view.tall() ? "Shorter preview" : "Taller preview"}
                        density="compact"
                        // The label says the state; no aria-pressed on top.
                        onClick={(e: MouseEvent) => {
                            e.stopPropagation();
                            props.view.setTall(!props.view.tall());
                        }}
                    />
                </Show>
            </div>
        </Show>
    );
}

/**
 * The body for a loaded result. Its height just before the result unloads is kept (the old nodes
 * are still in place when this branch is disposed), only when the result is what left: the row
 * going away (scrolled out, rolled off) needs no height. The unload pass reads every open body's
 * height first (body-height-cache.ts), so this is not a layout per row; reading here is for an
 * unload that did not go through it. A plain function for the same reason as `liveTail`.
 */
function loadedResultBody(props: ToolPanelProps, panelEl: () => HTMLElement | undefined): JSX.Element {
    onCleanup(() => {
        if (!props.node.resultUnloaded) return;
        const known = takeBodyHeight(props.node.id);
        if (known !== undefined) {
            props.unloaded.bodyPx.value = known;
            return;
        }
        const body = panelEl()?.querySelector<HTMLElement>(".agent-tool-overlay");
        props.unloaded.bodyPx.value = body?.offsetHeight ?? 0; // perf:allow-layout-read — an unload outside the batched pass
    });
    return (
        <ToolBlockOverlay
            node={props.node}
            dispatchMatch={props.dispatchMatch}
            open={props.view.bodyShowing()}
            dormant={props.view.dormant()}
            onOverflowChange={props.view.setScrolls}
        />
    );
}

/**
 * The panel: always rendered in the DOM so CSS transitions can animate the off→on shift; `inert`
 * and `aria-hidden` remove it from the focus/a11y tree when hidden. Two render modes: `flow`
 * (in-flow under the summary: pinned, running, post-hold) and `hidden`.
 */
export function ToolPanel(props: ToolPanelProps): JSX.Element {
    let panelEl: HTMLDivElement | undefined;
    const panelMode = (): "hidden" | "flow" => (props.view.expanded() ? "flow" : "hidden");
    return (
        <div
            ref={panelEl}
            class={clsx("agent-tool-panel", {
                "agent-tool-panel--hidden": panelMode() === "hidden",
                "agent-tool-panel--flow": panelMode() === "flow",
                "agent-tool-panel--tall": props.view.tall() && panelMode() === "flow",
            })}
            // The collapsed panel is visually hidden via max-height / opacity but would still be
            // in the focusable + a11y tree, so keyboard users could tab into action buttons that
            // aren't visible (#988). `inert` removes the entire subtree from focus and
            // accessibility while collapsed (Chrome 102+, supported in the bundled CEF runtime).
            inert={!props.view.expanded()}
            aria-hidden={!props.view.expanded()}
            onClick={(e) => e.stopPropagation()}
        >
            <Show when={props.view.bodyMounted()}>
                <ToolPanelActions node={props.node} view={props.view} />
                <Show
                    when={!props.node.resultUnloaded}
                    fallback={
                        // As tall as the body it stands in for, so a held row whose text left
                        // memory in a hidden pane doesn't shrink and move the rows below it
                        // while it reads back.
                        <div
                            class="agent-tool-result-unloaded"
                            style={props.unloaded.bodyPx.value > 0 ? { "min-height": `${props.unloaded.bodyPx.value}px` } : undefined}
                        >
                            {props.unloaded.loadState() === "failed"
                                ? "This result is no longer available in the pane. Open History to see it."
                                : "Loading result…"}
                        </div>
                    }
                >
                    {loadedResultBody(props, () => panelEl)}
                </Show>
            </Show>
        </div>
    );
}
