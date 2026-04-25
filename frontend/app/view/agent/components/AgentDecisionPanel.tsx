// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDecisionPanel — surfaced when one or more `ToolNode`s in the
 * pane have `status === "pending_approval"`. Lets the user allow or
 * deny a per-tool-call permission gate.
 *
 * Implements docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §5.
 *
 * **v1 PR-2 scope (this PR):** panel UI + decision callback
 * contract. The actual `tool:decision` IPC + sidecar stdin write
 * lands in PR-3; today the panel just calls
 * `props.onDecide(decision)` and the caller is responsible for
 * removing the pending node. Without a real-detection path
 * (PR-4), the panel never appears in production — but everything
 * downstream of "user clicked Allow / Deny" can be exercised.
 */

import { createMemo, createSignal, For, Show, type Accessor, type JSX } from "solid-js";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import type { PermissionRequestEvent, ToolNode } from "../types";

export interface DecisionOutcome {
    request_id: string;
    outcome: "allow" | "deny";
    scope: "once" | "session" | "project" | "global";
    feedback?: string;
}

interface AgentDecisionPanelProps {
    /** Pending decisions, oldest first. The panel shows the head; a
     *  chevron indicates how many more are queued. */
    pending: Accessor<ToolNode[]>;
    /** User decision. Caller is responsible for advancing the queue
     *  by removing the corresponding node from `pending`. */
    onDecide: (decision: DecisionOutcome) => void | Promise<void>;
    /** Defer — leave the prompt in pending state. Closes the panel
     *  without making a decision. */
    onDefer?: () => void;
}

const SCOPE_LABEL: Record<DecisionOutcome["scope"], string> = {
    once: "Once",
    session: "Session",
    project: "Project",
    global: "Global",
};

const SCOPE_HINT: Record<DecisionOutcome["scope"], string> = {
    once: "Just this one call",
    session: "Until this pane closes",
    project: "This repo, future runs",
    global: "Everywhere, future runs",
};

const SCOPE_ORDER: DecisionOutcome["scope"][] = ["once", "session", "project", "global"];

export const AgentDecisionPanel = (props: AgentDecisionPanelProps): JSX.Element => {
    let rootRef: HTMLDivElement | undefined;
    // Win32 airspace cut so the panel paints over any browser pane
    // HWND in the same window. Same primitive used by modal-v2 and
    // MoreDropdown — see frontend/app/platform/pane-overlay.ts.
    usePaneOverlay(() => rootRef);

    const head = createMemo<ToolNode | null>(() => props.pending()[0] ?? null);
    const queueDepth = () => props.pending().length;

    const request = (): PermissionRequestEvent | null => head()?.pendingPermission ?? null;
    const risk = (): "low" | "medium" | "high" => request()?.risk ?? "medium";

    const [scope, setScope] = createSignal<DecisionOutcome["scope"]>("once");
    const [feedback, setFeedback] = createSignal("");
    const [denyMode, setDenyMode] = createSignal(false);
    // High-risk Allow needs a second action (double-Enter or shift-click).
    // Tracked via a transient flag set on first activate; cleared after
    // 500ms so the user can't approve in their sleep.
    const [highRiskArmed, setHighRiskArmed] = createSignal(false);
    let highRiskTimer: number | null = null;

    const armHighRisk = () => {
        setHighRiskArmed(true);
        if (highRiskTimer != null) window.clearTimeout(highRiskTimer);
        highRiskTimer = window.setTimeout(() => {
            setHighRiskArmed(false);
            highRiskTimer = null;
        }, 500);
    };

    const dispatchAllow = (e?: KeyboardEvent | MouseEvent) => {
        const r = request();
        if (!r) return;
        if (risk() === "high") {
            const shiftHeld = e && "shiftKey" in e && e.shiftKey;
            if (!shiftHeld && !highRiskArmed()) {
                armHighRisk();
                return;
            }
        }
        void props.onDecide({
            request_id: r.request_id,
            outcome: "allow",
            scope: scope(),
        });
        setHighRiskArmed(false);
        if (highRiskTimer != null) {
            window.clearTimeout(highRiskTimer);
            highRiskTimer = null;
        }
    };

    const dispatchDeny = () => {
        const r = request();
        if (!r) return;
        const text = feedback().trim();
        // §10 open question: empty deny scopes only allowed for "once".
        // Pragmatic default: empty allowed for once, required for others.
        if (scope() !== "once" && text.length === 0) {
            // Keep deny mode open; rely on the textarea's required cue.
            return;
        }
        void props.onDecide({
            request_id: r.request_id,
            outcome: "deny",
            scope: scope(),
            feedback: text || undefined,
        });
        setDenyMode(false);
        setFeedback("");
    };

    const handleKey = (e: KeyboardEvent) => {
        if (e.key === "Escape") {
            e.preventDefault();
            props.onDefer?.();
            return;
        }
        // Letter-keyed scope only when not typing into the feedback textarea.
        const target = e.target as HTMLElement | null;
        const inFeedback = target?.tagName === "TEXTAREA";
        if (!inFeedback && e.key.length === 1) {
            const map: Record<string, DecisionOutcome["scope"]> = {
                o: "once", s: "session", p: "project", g: "global",
            };
            const sc = map[e.key.toLowerCase()];
            if (sc) {
                e.preventDefault();
                setScope(sc);
                return;
            }
        }
        if (e.key === "Enter") {
            if (denyMode()) {
                if (e.shiftKey || (target?.tagName !== "TEXTAREA")) {
                    e.preventDefault();
                    dispatchDeny();
                }
            } else if (e.shiftKey) {
                e.preventDefault();
                setDenyMode(true);
            } else {
                e.preventDefault();
                dispatchAllow(e);
            }
        }
    };

    const previewText = (): string | null => {
        const p = request()?.preview;
        if (!p) return null;
        switch (p.kind) {
            case "diff": return p.content;
            case "bash": return `$ ${p.command}`;
            case "text": return p.content;
            case "none": return null;
        }
    };

    return (
        <Show when={request()}>
            <div
                ref={rootRef}
                class="agent-decision-panel"
                classList={{
                    "agent-decision-panel--high-risk": risk() === "high",
                    "agent-decision-panel--armed": highRiskArmed(),
                }}
                role="dialog"
                aria-label="Permission decision required"
                onKeyDown={handleKey}
                tabIndex={-1}
            >
                <div class="agent-decision-panel-header">
                    <span class="agent-decision-panel-icon" aria-hidden="true">⚠</span>
                    <span class="agent-decision-panel-title">
                        Decision required
                    </span>
                    <Show when={queueDepth() > 1}>
                        <span class="agent-decision-panel-queue">
                            {queueDepth() - 1} more queued
                        </span>
                    </Show>
                    <Show when={risk() === "high"}>
                        <span class="agent-decision-panel-risk">high-risk</span>
                    </Show>
                </div>

                <dl class="agent-decision-panel-meta">
                    <dt>Tool</dt>
                    <dd>{request()!.tool}</dd>
                    <Show when={request()!.target}>
                        <dt>Target</dt>
                        <dd><code>{request()!.target}</code></dd>
                    </Show>
                    <Show when={request()!.reason}>
                        <dt>Why</dt>
                        <dd>{request()!.reason}</dd>
                    </Show>
                </dl>

                <Show when={previewText()}>
                    <pre class="agent-decision-panel-preview"><code>{previewText()}</code></pre>
                </Show>

                <fieldset class="agent-decision-panel-scope">
                    <legend>Scope</legend>
                    <For each={SCOPE_ORDER}>
                        {(s) => (
                            <label
                                class="agent-decision-panel-scope-option"
                                classList={{ "agent-decision-panel-scope-option--active": scope() === s }}
                            >
                                <input
                                    type="radio"
                                    name="agent-decision-scope"
                                    checked={scope() === s}
                                    onChange={() => setScope(s)}
                                />
                                <span class="agent-decision-panel-scope-label">{SCOPE_LABEL[s]}</span>
                                <span class="agent-decision-panel-scope-hint">{SCOPE_HINT[s]}</span>
                            </label>
                        )}
                    </For>
                </fieldset>

                <Show when={denyMode()}>
                    <label class="agent-decision-panel-feedback">
                        <span>Tell the agent why (sent verbatim on the next turn)</span>
                        <textarea
                            class="agent-decision-panel-feedback-input"
                            value={feedback()}
                            placeholder="e.g. don't delete my node_modules; run npm ci"
                            onInput={(e) => setFeedback(e.currentTarget.value)}
                            rows={3}
                            autofocus
                        />
                    </label>
                </Show>

                <div class="agent-decision-panel-actions">
                    <Show when={!denyMode()} fallback={
                        <>
                            <button
                                type="button"
                                class="agent-decision-panel-btn agent-decision-panel-btn--cancel"
                                onClick={() => { setDenyMode(false); setFeedback(""); }}
                            >
                                Back
                            </button>
                            <button
                                type="button"
                                class="agent-decision-panel-btn agent-decision-panel-btn--deny"
                                onClick={dispatchDeny}
                            >
                                Send denial
                            </button>
                        </>
                    }>
                        <button
                            type="button"
                            class="agent-decision-panel-btn agent-decision-panel-btn--allow"
                            classList={{ "agent-decision-panel-btn--armed": risk() === "high" && highRiskArmed() }}
                            onClick={(e) => dispatchAllow(e)}
                            title={
                                risk() === "high"
                                    ? "Hold Shift while clicking, or press Enter twice (high-risk confirm)"
                                    : "Allow this tool call"
                            }
                        >
                            {risk() === "high" && highRiskArmed() ? "Press again to allow" : "Allow"}
                        </button>
                        <button
                            type="button"
                            class="agent-decision-panel-btn agent-decision-panel-btn--deny"
                            onClick={() => setDenyMode(true)}
                        >
                            Deny + feedback
                        </button>
                        <Show when={props.onDefer}>
                            <button
                                type="button"
                                class="agent-decision-panel-btn agent-decision-panel-btn--cancel"
                                onClick={() => props.onDefer?.()}
                            >
                                Defer
                            </button>
                        </Show>
                    </Show>
                </div>
            </div>
        </Show>
    );
};

AgentDecisionPanel.displayName = "AgentDecisionPanel";
