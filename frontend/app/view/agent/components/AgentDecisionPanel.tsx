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

import { createEffect, createMemo, createSignal, createUniqueId, For, onCleanup, Show, type Accessor, type JSX } from "solid-js";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
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

// Tiny child that calls usePaneOverlay against the panel root ref.
// Same pattern as `<Modal>`'s `ModalPaneOverlayClip` — mounted only
// when the panel is visible, so `onMount` runs while `rootRef` is
// already attached to the live DOM element. Calling usePaneOverlay
// from the outer component would register undefined on first mount
// (the `<Show>` hasn't materialised yet) and never refresh on
// reveal — codex P1 on PR #556.
const DecisionPanelClip = (p: { getEl: Accessor<HTMLElement | null | undefined> }): JSX.Element => {
    usePaneOverlay(p.getEl);
    return null;
};

export const AgentDecisionPanel = (props: AgentDecisionPanelProps): JSX.Element => {
    // Holds either the expanded panel div or the minimized button
    // depending on which branch is mounted; widened to HTMLElement
    // so both ref assignments type-check.
    let rootRef: HTMLElement | undefined;
    // Per-instance unique IDs. Anything that's `name=` on a radio,
    // `id=` on an element targeted by aria-describedby/-labelledby,
    // or otherwise globally addressable in the DOM must be scoped
    // per-instance. Hardcoding caused multi-pane desync — reagent
    // P2 round-5 (radio name) + round-6 (deny-error id).
    const uid = createUniqueId();
    const scopeGroupName = `agent-decision-scope-${uid}`;
    const denyErrorId = `agent-decision-deny-error-${uid}`;

    const head = createMemo<ToolNode | null>(() => props.pending()[0] ?? null);
    const queueDepth = () => props.pending().length;

    // Minimized state — Defer / Esc collapses the panel into a
    // compact "Decision pending" bar instead of removing it from
    // the UI. Codex P1 round-4: previously the parent maintained a
    // deferredIds set that it never cleared, leaving deferred
    // prompts permanently unreachable. Single per-request reset
    // effect below clears this and every other transient signal
    // when the head changes — see SPEC_DECISION_PROMPT_DESIGN §2.
    const [minimized, setMinimized] = createSignal(false);

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

    // Visible validation message when Send-denial is pressed without
    // the required feedback. Cleared on next keystroke in the textarea
    // (and on per-request reset below).
    const [denyError, setDenyError] = createSignal<string | null>(null);

    const armHighRisk = () => {
        setHighRiskArmed(true);
        if (highRiskTimer != null) window.clearTimeout(highRiskTimer);
        highRiskTimer = window.setTimeout(() => {
            setHighRiskArmed(false);
            highRiskTimer = null;
        }, 500);
    };

    // SINGLE per-request reset for every transient panel signal. Per
    // SPEC_DECISION_PROMPT_DESIGN §2 invariant: when the head request
    // changes (defer advances queue, deny resolves, new prompt
    // arrives), every per-prompt UI flag must clear so we never
    // accidentally inherit state from the previous request — most
    // importantly the high-risk armed flag (otherwise arming prompt
    // A and advancing to B within 500ms auto-commits B).
    createEffect(() => {
        const id = request()?.request_id ?? null;
        // touch id so the effect re-runs on every change
        void id;
        setMinimized(false);
        setScope("once");
        setDenyMode(false);
        setFeedback("");
        setDenyError(null);
        setHighRiskArmed(false);
        if (highRiskTimer != null) {
            window.clearTimeout(highRiskTimer);
            highRiskTimer = null;
        }
    });

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
            setDenyError("Feedback is required when the denial applies beyond just this call.");
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
        setDenyError(null);
    };

    const isEditableTarget = (target: EventTarget | null): boolean => {
        const el = target as HTMLElement | null;
        if (!el) return false;
        const tag = el.tagName;
        if (tag === "INPUT" || tag === "TEXTAREA") return true;
        if (el.isContentEditable) return true;
        return false;
    };

    const handleKey = (e: KeyboardEvent) => {
        const target = e.target as HTMLElement | null;
        // Scope every shortcut to events originating inside this
        // panel's own pane. Without this check, a pending prompt in
        // pane A would react to keys from pane B (Esc, Enter, etc.)
        // and multiple open prompts in different panes would all
        // dispatch on the same keystroke. Codex P1 on PR #556.
        const paneRoot = rootRef?.closest(".agent-view") as HTMLElement | null;
        if (paneRoot && target && !paneRoot.contains(target)) return;

        const inPanel = !!rootRef && !!target && rootRef.contains(target);
        const editable = isEditableTarget(target);
        const inFeedback = inPanel && target?.tagName === "TEXTAREA";

        if (e.key === "Escape") {
            // Esc minimizes ONLY when focus is non-editable or in the
            // panel itself. Reagent P1 round-3: Esc in the composer
            // textarea (or any editable in the pane) needs to keep its
            // normal meaning — dismiss autocomplete, cancel slash
            // picker, etc.
            if (!editable || inPanel) {
                e.preventDefault();
                e.stopPropagation();
                setMinimized(true);
                props.onDefer?.();
            }
            return;
        }
        // Letter-keyed scope: only when not typing into any editable
        // (composer, feedback textarea, etc.). Composer keystrokes
        // would otherwise be hijacked.
        if (!editable && e.key.length === 1) {
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
            // Inside the deny feedback textarea: Enter inserts a newline
            // (default browser behaviour); Shift+Enter sends the denial.
            if (inFeedback) {
                if (e.shiftKey) {
                    e.preventDefault();
                    dispatchDeny();
                }
                return;
            }
            // Outside any editable element OR inside the panel itself:
            // Enter = Allow (or arm high-risk), Shift+Enter = enter deny mode.
            if (!editable || inPanel) {
                if (denyMode()) {
                    e.preventDefault();
                    dispatchDeny();
                } else if (e.shiftKey) {
                    e.preventDefault();
                    setDenyMode(true);
                } else {
                    e.preventDefault();
                    dispatchAllow(e);
                }
            }
            // If editable and not in panel (composer): let the composer
            // handle Enter. The user must click the panel to act on it.
        }
    };

    // Install a global capture-phase keydown listener while the panel
    // is open so decision shortcuts work even when focus is elsewhere
    // (e.g. the composer textarea). Codex P1 on PR #556: previously the
    // panel's `onKeyDown` only fired when the panel itself was focused,
    // but the panel has tabIndex=-1 and never auto-focuses, so keys
    // never reached the handler in practice.
    createEffect(() => {
        if (!request()) return;
        const onWindowKey = (e: KeyboardEvent) => handleKey(e);
        window.addEventListener("keydown", onWindowKey, true);
        onCleanup(() => window.removeEventListener("keydown", onWindowKey, true));
    });

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
            <Show
                when={!minimized()}
                fallback={
                    <button
                        type="button"
                        ref={(el) => { rootRef = el; }}
                        class="agent-decision-panel-minimized"
                        classList={{ "agent-decision-panel-minimized--high-risk": risk() === "high" }}
                        onClick={() => setMinimized(false)}
                        aria-label="Re-open decision prompt"
                    >
                        <DecisionPanelClip getEl={() => rootRef} />
                        <span class="agent-decision-panel-icon" aria-hidden="true">⚠</span>
                        <span>
                            Decision pending — {request()!.tool}
                            <Show when={queueDepth() > 1}> ({queueDepth()} total)</Show>
                        </span>
                        <span class="agent-decision-panel-minimized-cta">click to decide</span>
                    </button>
                }
            >
            <div
                ref={(el) => { rootRef = el; }}
                class="agent-decision-panel"
                classList={{
                    "agent-decision-panel--high-risk": risk() === "high",
                    "agent-decision-panel--armed": highRiskArmed(),
                }}
                role="dialog"
                aria-label="Permission decision required"
                tabIndex={-1}
            >
                <DecisionPanelClip getEl={() => rootRef} />
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
                                    name={scopeGroupName}
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
                            classList={{ "agent-decision-panel-feedback-input--error": denyError() != null }}
                            value={feedback()}
                            placeholder="e.g. don't delete my node_modules; run npm ci"
                            onInput={(e) => {
                                setFeedback(e.currentTarget.value);
                                if (denyError()) setDenyError(null);
                            }}
                            onContextMenu={showTextInputContextMenu}
                            rows={3}
                            autofocus
                            aria-invalid={denyError() != null}
                            aria-describedby={denyError() ? denyErrorId : undefined}
                        />
                        <Show when={denyError()}>
                            <span
                                id={denyErrorId}
                                class="agent-decision-panel-feedback-error"
                                role="alert"
                            >
                                {denyError()}
                            </span>
                        </Show>
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
                                onClick={() => {
                                    // Symmetric with Esc handler: minimize the
                                    // panel into the compact "Decision pending"
                                    // bar, then notify parent (logging only).
                                    // Reagent P1 round-5 — without setMinimized,
                                    // the click was a visible no-op.
                                    setMinimized(true);
                                    props.onDefer?.();
                                }}
                            >
                                Defer
                            </button>
                        </Show>
                    </Show>
                </div>
            </div>
            </Show>
        </Show>
    );
};

AgentDecisionPanel.displayName = "AgentDecisionPanel";
