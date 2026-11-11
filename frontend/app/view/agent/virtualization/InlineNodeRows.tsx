// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The transcript rows that DocumentRow draws itself, each its own component: the error row, the
 * compaction, memory, day, session and resume rows, the History link and the CLI notices. The
 * rows that carry a hover peek share the one instance DocumentNodeBody makes (only one kind is ever
 * mounted for a node, so one is enough).
 */

import { createMemo, Show, type Accessor, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { CopyErrorButton } from "@/app/errors/CopyErrorButton";
import { formatErrorReport } from "@/app/errors/error-report";
import { showCopyContextMenu } from "@/app/store/contextmenu";
import { getApi } from "@/app/store/global";
import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatExactTime, formatPeekTime } from "@/util/format-time";
import { historyLinkLabel } from "../../../agent-model/live-feed";
import { sessionOutcomeDetail, sessionOutcomeLabel } from "../../../agent-model/session-outcome-text";
import type { DocumentNode, HistoryLinkNode } from "../../../agent-model/types";
import { cliNoticeText } from "../cli-notice";
import { compactionCardTokens } from "../compact-boundary";
import { ContainerLimitButton } from "../components/ContainerLimitButton";
import { ContainerUpdateButton } from "../components/ContainerUpdateButton";
import { PeekOverlay } from "../components/PeekOverlay";

/** The hover peek the rows share, and the tick that keeps its time-ago current. */
export interface RowPeek {
    isPeeking: Accessor<boolean>;
    peekPanelVisible: Accessor<boolean>;
    peekRowEl: Accessor<HTMLElement | undefined>;
    setPeekRowEl: (el: HTMLElement) => void;
    handlePeekEnter: () => void;
    handlePeekLeave: () => void;
    peekTick: Accessor<number>;
}

/** The version of the AgentMux showing the conversation; a persisted row may name an older one. */
function runningAppVersion(): string | undefined {
    try {
        return getApi().getAboutModalDetails().version;
    } catch {
        return undefined;
    }
}


interface AgentErrorRowProps {
    node: Accessor<DocumentNode>;
    peek: RowPeek;
    onAgentErrorLogin?: () => void;
}

export function AgentErrorRow(props: AgentErrorRowProps): JSX.Element {
    // Read through props.node() at each use, as DocumentNodeBody does: the row follows its node.
    const { isPeeking, peekRowEl, setPeekRowEl, handlePeekEnter, handlePeekLeave } = props.peek;
    return (
        <>
            <div
                class="agent-error-block"
                classList={{ "agent-error-block--resolved": !!(props.node() as Extract<DocumentNode, { type: "agent_error" }>).resolved }}
                ref={setPeekRowEl}
                onMouseEnter={handlePeekEnter}
                onMouseLeave={handlePeekLeave}
                // SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md surface 3 —
                // this row is the only thing with something to copy;
                // the pane's own generic selection-copy menu never fires
                // here since nothing is selected on right-click.
                onContextMenu={(e) => {
                    const n = props.node() as Extract<DocumentNode, { type: "agent_error" }>;
                    showCopyContextMenu(
                        [{ label: "Copy error", value: formatErrorReport({ title: n.code > 0 ? `HTTP ${n.code}` : "Error", message: n.message }) }],
                        e
                    );
                }}
            >
                <span class="agent-error-code">
                    {(() => {
                        const n = props.node() as Extract<DocumentNode, { type: "agent_error" }>;
                        // code=0 is a sentinel for non-HTTP errors (network/CLI); don't show "HTTP 0"
                        return n.code > 0 ? `HTTP ${n.code}` : "Error";
                    })()}
                </span>
                <span class="agent-error-message">
                    {(props.node() as Extract<DocumentNode, { type: "agent_error" }>).message}
                </span>
                <Show when={(props.node() as Extract<DocumentNode, { type: "agent_error" }>).resolved}>
                    <span class="agent-error-resolved">signed in since</span>
                </Show>
                {/* An auth error (401/403) is recoverable by logging in again: an inline button drives the
                    same flow as the failure banner (SPEC_REAUTH_FROM_AUTH_ERROR §7). Kept beside that
                    banner on purpose: the banner is transient pane state, this node stays in the
                    transcript, and scrolling back to an old 401 is the case it serves. A turn did run
                    and was rejected, so the handler takes relogin()'s default `retryAfterLogin: true`. */}
                <Show when={
                    props.onAgentErrorLogin
                    && !(props.node() as Extract<DocumentNode, { type: "agent_error" }>).resolved
                    && [401, 403].includes((props.node() as Extract<DocumentNode, { type: "agent_error" }>).code)
                }>
                    <Button tone="danger" density="compact" class="agent-error-login-btn" onClick={() => props.onAgentErrorLogin?.()}>
                        Login Again →
                    </Button>
                </Show>
                {/* SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md surface 3 —
                    icon at the row's end, shown on hover/focus. */}
                <CopyErrorButton
                    variant="icon"
                    className="agent-error-copy"
                    report={() => {
                        const n = props.node() as Extract<DocumentNode, { type: "agent_error" }>;
                        return formatErrorReport({ title: n.code > 0 ? `HTTP ${n.code}` : "Error", message: n.message });
                    }}
                />
                {(() => {
                    // No timestamp field exists on AgentErrorNode — estimate only,
                    // same "no time line" shape ToolBlock/MarkdownBlock use for an
                    // untimed node.
                    const n = props.node() as Extract<DocumentNode, { type: "agent_error" }>;
                    const estimateText = createMemo(() => {
                        const count = estimateTokenCount(n.message);
                        return count > 0 ? `~${formatCompactNumber(count)} tok (est.)` : null;
                    });
                    return (
                        <PeekOverlay show={isPeeking() && estimateText() != null} rowEl={peekRowEl}>
                            <Show when={estimateText()}>
                                <div class="agent-node-peek-tooltip-meta">{estimateText()}</div>
                            </Show>
                        </PeekOverlay>
                    );
                })()}
            </div>
        </>
    );
}

interface ContextCompactedRowProps {
    node: Accessor<DocumentNode>;
    peek: RowPeek;
}

export function ContextCompactedRow(props: ContextCompactedRowProps): JSX.Element {
    // Read through props.node() at each use, as DocumentNodeBody does: the row follows its node.
    const { isPeeking, peekPanelVisible, peekRowEl, setPeekRowEl, handlePeekEnter, handlePeekLeave, peekTick } = props.peek;
    return (
        <>
            {(() => {
                const n = props.node() as Extract<DocumentNode, { type: "context_compacted" }>;
                const fmt = formatCompactNumber;
                // Real events (backend `CompactionBoundary`) carry a real
                // trigger + duration; the heuristic fallback (other
                // providers, or a missed real event) has neither. See
                // docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.3.
                const triggerLabel = n.trigger === "manual"
                    ? "you ran /compact"
                    : n.trigger === "auto"
                        ? "auto-compacted"
                        : null;
                const durationLabel = n.durationMs != null
                    ? ` · took ${(n.durationMs / 1000).toFixed(1)}s`
                    : "";
                // Only a time line — tokensBefore/After/duration are already
                // visible in the row itself, and there's no free-text body to
                // estimate a token count from.
                const timeText = createMemo(() => {
                    if (!peekPanelVisible()) return null;
                    peekTick();
                    return formatPeekTime(n.timestamp);
                });
                return (
                    <div
                        ref={setPeekRowEl}
                        class="agent-context-compacted"
                        onMouseEnter={handlePeekEnter}
                        onMouseLeave={handlePeekLeave}
                    >
                        <div class="agent-context-compacted-rule">
                            <span class="agent-context-compacted-label">
                                context compacted{triggerLabel ? ` — ${triggerLabel}` : ""}
                            </span>
                        </div>
                        <div class="agent-context-compacted-detail">
                            Earlier history summarized · {compactionCardTokens(n, fmt)}{durationLabel}
                        </div>
                        <PeekOverlay show={isPeeking() && timeText() != null} rowEl={peekRowEl}>
                            <div class="agent-node-peek-tooltip-meta">{timeText()}</div>
                        </PeekOverlay>
                    </div>
                );
            })()}
        </>
    );
}

interface CompactionStartedRowProps {
    node: Accessor<DocumentNode>;
    peek: RowPeek;
}

export function CompactionStartedRow(props: CompactionStartedRowProps): JSX.Element {
    // Read through props.node() at each use, as DocumentNodeBody does: the row follows its node.
    const { isPeeking, peekPanelVisible, peekRowEl, setPeekRowEl, handlePeekEnter, handlePeekLeave, peekTick } = props.peek;
    return (
        <>
            {(() => {
                const n = props.node() as Extract<DocumentNode, { type: "compaction_started" }>;
                const triggerLabel = n.trigger === "manual" ? "you ran /compact" : "context filled up";
                // Node uses `startedAt`, not `timestamp` — same time-only peek
                // shape as context_compacted above.
                const timeText = createMemo(() => {
                    if (!peekPanelVisible()) return null;
                    peekTick();
                    return formatPeekTime(n.startedAt);
                });
                return (
                    <div
                        ref={setPeekRowEl}
                        class="agent-compaction-started"
                        onMouseEnter={handlePeekEnter}
                        onMouseLeave={handlePeekLeave}
                    >
                        <div class="agent-compaction-started-label">
                            Compacting conversation…
                        </div>
                        <div class="agent-compaction-started-detail">
                            {triggerLabel}
                        </div>
                        <PeekOverlay show={isPeeking() && timeText() != null} rowEl={peekRowEl}>
                            <div class="agent-node-peek-tooltip-meta">{timeText()}</div>
                        </PeekOverlay>
                    </div>
                );
            })()}
        </>
    );
}

interface MemoryReinjectionRowProps {
    node: Accessor<DocumentNode>;
    peek: RowPeek;
}

export function MemoryReinjectionRow(props: MemoryReinjectionRowProps): JSX.Element {
    // Read through props.node() at each use, as DocumentNodeBody does: the row follows its node.
    const { isPeeking, peekPanelVisible, peekRowEl, setPeekRowEl, handlePeekEnter, handlePeekLeave, peekTick } = props.peek;
    return (
        <>
            {(() => {
                // Label-only row — deliberately never carries the actual
                // memory body text; that content only ever exists in the
                // real message already sent to the CLI, never in this
                // node's own data. See
                // docs/specs/SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md
                // §3.2.
                const n = props.node() as Extract<DocumentNode, { type: "memory_reinjection" }>;
                const fmt = formatCompactNumber;
                const countLabel = `${n.globalMemoryCount} global, ${n.personalMemoryCount} personal`;
                // Per-entry breakdown + real byte totals — "the token
                // count of each" (§4) surfaced on hover, same PeekOverlay
                // pattern the other compaction-family nodes already use
                // above rather than a second, different disclosure UI.
                const breakdownText = createMemo(() => {
                    if (!peekPanelVisible()) return null;
                    peekTick();
                    const lines = n.perEntryTokens.map(
                        (e) => `${e.source === "global" ? "🌐" : "👤"} ${e.label} — ~${fmt(e.tokens)} tok (est.), ${fmt(e.sizeBytes)} B`,
                    );
                    const totalsLine = `Total: ${fmt(n.totalSizeBytes.global)} B global, ${fmt(n.totalSizeBytes.personal)} B personal`;
                    // Informational suggestion text only, §3.4.3 — no
                    // action affordance (compress button / "create a
                    // WorkItem" click) is wired here. §3.4.4 leaves
                    // whether that action ever auto-fires (vs. requires
                    // explicit confirmation) as an open product
                    // decision; building an interactive trigger for an
                    // unresolved interaction would mean guessing at it.
                    // Text-only guidance carries none of that risk.
                    const suggestionLines =
                        n.sizeBand === "high" || n.sizeBand === "critical"
                            ? [
                                  "",
                                  "Personal memory has grown large. Consider:",
                                  "• Compress: Memory list → consolidate with Memory write (check its history and diff first)",
                                  "• Delegate: hand the next segment of work to another agent via Tasks enqueue",
                              ]
                            : [];
                    return [...lines, totalsLine, ...suggestionLines].join("\n");
                });
                return (
                    <div
                        ref={setPeekRowEl}
                        class={`agent-memory-reinjection agent-memory-reinjection--${n.sizeBand}`}
                        onMouseEnter={handlePeekEnter}
                        onMouseLeave={handlePeekLeave}
                    >
                        <div class="agent-memory-reinjection-label">
                            🧠 Memory reinjected — {countLabel} (~{fmt(n.estimatedTokens)} tok, est.)
                        </div>
                        <PeekOverlay show={isPeeking() && breakdownText() != null} rowEl={peekRowEl}>
                            {/* white-space: pre-line (see scss) renders the \n-joined
                                per-entry lines without needing Solid's <For>, which
                                this file doesn't otherwise import. */}
                            <div class="agent-node-peek-tooltip-meta agent-memory-reinjection-tooltip">
                                {breakdownText()}
                            </div>
                        </PeekOverlay>
                    </div>
                );
            })()}
        </>
    );
}

interface DayDividerRowProps {
    node: Accessor<DocumentNode>;
    peek: RowPeek;
}

export function DayDividerRow(props: DayDividerRowProps): JSX.Element {
    // Read through props.node() at each use, as DocumentNodeBody does: the row follows its node.
    const { isPeeking, peekPanelVisible, peekRowEl, setPeekRowEl, handlePeekEnter, handlePeekLeave, peekTick } = props.peek;
    return (
        <>
            {(() => {
                const n = props.node() as Extract<DocumentNode, { type: "day_divider" }>;
                // Exact local-midnight instant — the visible label is already a
                // human day name, so this just adds precision, no estimate line
                // (a day label has no free-text body worth estimating).
                const timeText = createMemo(() => {
                    if (!peekPanelVisible()) return null;
                    peekTick();
                    return formatPeekTime(n.timestamp);
                });
                return (
                    <div
                        ref={setPeekRowEl}
                        class="agent-day-divider"
                        onMouseEnter={handlePeekEnter}
                        onMouseLeave={handlePeekLeave}
                    >
                        <div class="agent-day-divider-label">{n.dayLabel}</div>
                        <PeekOverlay show={isPeeking() && timeText() != null} rowEl={peekRowEl}>
                            <div class="agent-node-peek-tooltip-meta">{timeText()}</div>
                        </PeekOverlay>
                    </div>
                );
            })()}
        </>
    );
}

interface HistoryLinkRowProps {
    node: Accessor<DocumentNode>;
    onOpenHistory?: () => void;
}

export function HistoryLinkRow(props: HistoryLinkRowProps): JSX.Element {
    // Read through props.node() at each use, as DocumentNodeBody does: the row follows its node.
    return (
        <>
            <div
                class="agent-history-link-row"
                role="button"
                tabindex="0"
                onClick={() => props.onOpenHistory?.()}
                onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        props.onOpenHistory?.();
                    }
                }}
            >
                <span class="agent-history-link-sigil">⌛</span>
                <span class="agent-history-link-label">{historyLinkLabel(props.node() as HistoryLinkNode)}</span>
                <span class="agent-history-link-cta">Open Agent History →</span>
            </div>
        </>
    );
}

interface ResumePreflightRowProps {
    node: Accessor<DocumentNode>;
    onOpenHistory?: () => void;
}

export function ResumePreflightRow(props: ResumePreflightRowProps): JSX.Element {
    // Read through props.node() at each use, as DocumentNodeBody does: the row follows its node.
    return (
        <>
            {(() => {
                const n = props.node() as Extract<DocumentNode, { type: "resume_preflight" }>;
                return (
                    <Show
                        when={!n.pending}
                        fallback={
                            // Only ever rendered when the check has already
                            // run longer than its reveal delay — see
                            // `useResumePreflight`'s STEPS_VISIBLE_AFTER_MS.
                            <div class="agent-resume-preflight agent-resume-preflight-pending">
                                <span class="agent-resume-preflight-sigil">⟳</span>
                                <span class="agent-resume-preflight-label">
                                    Checking whether this conversation can resume…
                                </span>
                            </div>
                        }
                    >
                        <div
                            class={
                                n.verdict === "recover"
                                    ? "agent-resume-preflight agent-resume-preflight-recover"
                                    : "agent-resume-preflight agent-resume-preflight-fresh"
                            }
                        >
                            <span class="agent-resume-preflight-sigil">
                                {n.verdict === "recover" ? "⟲" : "⚠"}
                            </span>
                            <span class="agent-resume-preflight-label">
                                {n.verdict === "recover"
                                    ? "This agent will reconnect to its conversation — it may pause briefly on your first message."
                                    : "This agent will start a new conversation — the history below isn't available to it."}
                            </span>
                            <Show when={n.verdict === "fresh" && props.onOpenHistory}>
                                <span
                                    class="agent-resume-preflight-cta"
                                    role="button"
                                    tabindex="0"
                                    onClick={() => props.onOpenHistory?.()}
                                    onKeyDown={(e) => {
                                        if (e.key === "Enter" || e.key === " ") {
                                            e.preventDefault();
                                            props.onOpenHistory?.();
                                        }
                                    }}
                                >
                                    Open Agent History →
                                </span>
                            </Show>
                        </div>
                    </Show>
                );
            })()}
        </>
    );
}

interface SessionOutcomeRowProps {
    node: Accessor<DocumentNode>;
    peek: RowPeek;
}

export function SessionOutcomeRow(props: SessionOutcomeRowProps): JSX.Element {
    // Read through props.node() at each use, as DocumentNodeBody does: the row follows its node.
    const { isPeeking, peekPanelVisible, peekRowEl, setPeekRowEl, handlePeekEnter, handlePeekLeave, peekTick } = props.peek;
    return (
        <>
            {(() => {
                const n = props.node() as Extract<DocumentNode, { type: "session_outcome" }>;
                // `resumed` nodes are no longer materialized in the
                // working document (§3.5 of the session-scoped-scrollback
                // spec — both parse paths skip them), so this branch only
                // sees `fresh` today. The resumed rendering is kept for
                // the P2 Agent History view, which materializes both.
                const resumed = n.outcome === "resumed";
                // Body shows the attempted vs. actual session id — real
                // debug info this node's own visible label doesn't surface
                // anywhere else.
                const timeText = createMemo(() => {
                    if (!peekPanelVisible()) return null;
                    peekTick();
                    return formatPeekTime(n.timestamp);
                });
                // An empty `attemptedSid` is meaningful, not missing: the
                // spawn had no session id to resume at all (srv's
                // `fresh_start_needs_disclosure` path), as opposed to
                // having attempted one that was rejected. Render it as "—"
                // so the peek body doesn't read as a truncated id.
                const bodyText = `attempted: ${n.attemptedSid || "—"} · actual: ${n.actualSid ?? "—"}`;
                return (
                    <div
                        ref={setPeekRowEl}
                        class={resumed ? "agent-session-outcome" : "agent-session-outcome agent-session-outcome-fresh"}
                        onMouseEnter={handlePeekEnter}
                        onMouseLeave={handlePeekLeave}
                    >
                        <div class="agent-session-outcome-rule">
                            <span class="agent-session-outcome-label">{sessionOutcomeLabel(n)}</span>
                        </div>
                        <Show when={!resumed}>
                            <div class="agent-session-outcome-detail">{sessionOutcomeDetail(n)}</div>
                        </Show>
                        <PeekOverlay show={isPeeking()} rowEl={peekRowEl}>
                            <Show when={timeText()}>
                                <div class="agent-node-peek-tooltip-meta">{timeText()}</div>
                            </Show>
                            <div class="agent-node-peek-tooltip-body">{bodyText}</div>
                        </PeekOverlay>
                    </div>
                );
            })()}
        </>
    );
}

interface CliNoticeRowProps {
    node: Accessor<DocumentNode>;
}

export function CliNoticeRow(props: CliNoticeRowProps): JSX.Element {
    // Read through props.node() at each use, as DocumentNodeBody does: the row follows its node.
    return (
        <>
            {(() => {
                // Read through the accessor, not captured once: an
                // install row is updated in place (installing → installed).
                const node = () => props.node() as Extract<DocumentNode, { type: "cli_notice" }>;
                const text = createMemo(() => cliNoticeText(node(), runningAppVersion()));
                return (
                    <div class={`agent-cli-notice agent-cli-notice-${text().tone}`} {...tip(formatExactTime(node().timestamp))}>
                        <div class="agent-cli-notice-rule">
                            <span class="agent-cli-notice-label">{text().label}</span>
                        </div>
                        <Show when={text().detail}>
                            <div class="agent-cli-notice-detail">{text().detail}</div>
                        </Show>
                        <Show when={node().kind === "container_image" && node().imageState === "outdated" && node().blockId}>
                            <ContainerUpdateButton blockId={node().blockId!} />
                        </Show>
                        <Show when={node().kind === "container_limit" && node().limitState === "oom" && node().agentId}>
                            <ContainerLimitButton agentId={node().agentId!} agentName={node().agentName || "this agent"} />
                        </Show>
                        <Show when={node().kind === "container_caches" && node().agentId}>
                            <ContainerLimitButton agentId={node().agentId!} agentName={node().agentName || "this agent"} label="Container settings" />
                        </Show>
                    </div>
                );
            })()}
        </>
    );
}
