// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What AgentDecisionPanel draws: the full panel, or its minimized bar. */

import { For, Show, type Accessor, type JSX } from "solid-js";
import { Button } from "@/app/element/ui/Button";
import { TextArea } from "@/app/element/ui/inputs";
import clsx from "clsx";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { focusOnOpen } from "@/util/focusutil";
import { SCOPE_HINT, SCOPE_LABEL, SCOPE_ORDER } from "./decision-panel-model";
import type { DecisionActions } from "./decision-panel-actions";
import type { DecisionState } from "./decision-panel-state";

export interface DecisionViewProps {
    s: DecisionState;
    d: DecisionActions;
}

// Tiny child that calls usePaneOverlay against the panel root ref.
// Same pattern as `<Modal>`'s `ModalPaneOverlayClip` — mounted only
// when the panel is visible, so `onMount` runs while `rootRef` is
// already attached to the live DOM element. Calling usePaneOverlay
// from the outer component would register undefined on first mount
// (the `<Show>` hasn't materialised yet) and never refresh on
// reveal (#556).
const DecisionPanelClip = (p: { getEl: Accessor<HTMLElement | null | undefined> }): JSX.Element => {
    usePaneOverlay(p.getEl);
    return null;
};

function MinimizedBar(props: DecisionViewProps): JSX.Element {
    const s = props.s;
    return (
        <Button
            variant="row"
            tone={s.risk() === "high" ? "danger" : "attention"}
            ref={(el) => {
                s.rootEl = el;
            }}
            class="agent-decision-panel-minimized"
            onClick={() => s.setMinimized(false)}
            aria-label="Re-open decision prompt"
        >
            <DecisionPanelClip getEl={() => s.rootEl} />
            <span class="agent-decision-panel-icon" aria-hidden="true">⚠</span>
            <span>
                Decision pending — {s.request()!.tool}
                <Show when={s.queueDepth() > 1}> ({s.queueDepth()} total)</Show>
            </span>
            <span class="agent-decision-panel-minimized-cta">click to decide</span>
        </Button>
    );
}

function DecisionHeader(props: DecisionViewProps): JSX.Element {
    const s = props.s;
    return (
        <div class="agent-decision-panel-header">
            <span class="agent-decision-panel-icon" aria-hidden="true">⚠</span>
            <span class="agent-decision-panel-title">
                Decision required
            </span>
            <Show when={s.queueDepth() > 1}>
                <span class="agent-decision-panel-queue">
                    {s.queueDepth() - 1} more queued
                </span>
            </Show>
            <Show when={s.risk() === "high"}>
                <span class="agent-decision-panel-risk">high-risk</span>
            </Show>
        </div>
    );
}

function DecisionDetails(props: DecisionViewProps): JSX.Element {
    const s = props.s;
    const previewText = (): string | null => {
        const p = s.request()?.preview;
        if (!p) return null;
        switch (p.kind) {
            case "diff": return p.content;
            case "bash": return `$ ${p.command}`;
            case "text": return p.content;
            case "none": return null;
        }
    };
    return (
        <>
            <dl class="agent-decision-panel-meta">
                <dt>Tool</dt>
                <dd>{s.request()!.tool}</dd>
                <Show when={s.request()!.target}>
                    <dt>Target</dt>
                    <dd><code>{s.request()!.target}</code></dd>
                </Show>
                <Show when={s.request()!.reason}>
                    <dt>Why</dt>
                    <dd>{s.request()!.reason}</dd>
                </Show>
            </dl>

            <Show when={previewText()}>
                <pre class="agent-decision-panel-preview"><code>{previewText()}</code></pre>
            </Show>
        </>
    );
}

function ScopeFieldset(props: DecisionViewProps): JSX.Element {
    const s = props.s;
    return (
        <fieldset class="agent-decision-panel-scope">
            <legend>Scope</legend>
            <For each={SCOPE_ORDER}>
                {(sc) => (
                    <label
                        class="agent-decision-panel-scope-option"
                        classList={{ "agent-decision-panel-scope-option--active": s.scope() === sc }}
                    >
                        <input
                            type="radio"
                            name={s.scopeGroupName}
                            checked={s.scope() === sc}
                            onChange={() => s.setScope(sc)}
                        />
                        <span class="agent-decision-panel-scope-label">{SCOPE_LABEL[sc]}</span>
                        <span class="agent-decision-panel-scope-hint">{SCOPE_HINT[sc]}</span>
                    </label>
                )}
            </For>
        </fieldset>
    );
}

function DenyFeedback(props: DecisionViewProps): JSX.Element {
    const s = props.s;
    return (
        <label class="agent-decision-panel-feedback">
            <span>Tell the agent why (sent verbatim on the next turn)</span>
            <TextArea
                variant="plain"
                class={clsx("agent-decision-panel-feedback-input", s.denyError() != null && "agent-decision-panel-feedback-input--error")}
                value={s.feedback()}
                placeholder="e.g. don't delete my node_modules; run npm ci"
                onInput={(e) => {
                    s.setFeedback(e.currentTarget.value);
                    if (s.denyError()) s.setDenyError(null);
                }}
                onContextMenu={showTextInputContextMenu}
                rows={3}
                ref={(el) => focusOnOpen(el)}
                aria-invalid={s.denyError() != null}
                aria-describedby={s.denyError() ? s.denyErrorId : undefined}
            />
            <Show when={s.denyError()}>
                <span
                    id={s.denyErrorId}
                    class="agent-decision-panel-feedback-error"
                    role="alert"
                >
                    {s.denyError()}
                </span>
            </Show>
        </label>
    );
}

function DecisionActionsRow(props: DecisionViewProps): JSX.Element {
    const { s, d } = props;
    return (
        <div class="agent-decision-panel-actions">
            <Show when={!s.denyMode()} fallback={
                <>
                    <Button
                        onClick={() => { s.setDenyMode(false); s.setFeedback(""); }}
                    >
                        Back
                    </Button>
                    <Button tone="danger" onClick={d.dispatchDeny}>
                        Send denial
                    </Button>
                </>
            }>
                <Button
                    tone={s.risk() === "high" ? "accent" : "attention"}
                    pressed={s.risk() === "high" ? s.highRiskArmed() : undefined}
                    onClick={(e) => d.dispatchAllow(e)}
                    title={
                        s.risk() === "high"
                            ? "Hold Shift while clicking, or press Enter twice (high-risk confirm)"
                            : "Allow this tool call"
                    }
                >
                    {s.risk() === "high" && s.highRiskArmed() ? "Press again to allow" : "Allow"}
                </Button>
                <Button tone="danger" onClick={() => s.setDenyMode(true)}>
                    Deny + feedback
                </Button>
                <Show when={s.props.onDefer}>
                    <Button
                        onClick={() => {
                            // Symmetric with Esc handler: minimize the
                            // panel into the compact "Decision pending"
                            // bar, then notify parent (logging only).
                            // Without setMinimized, the click was a visible no-op.
                            s.setMinimized(true);
                            s.props.onDefer?.();
                        }}
                    >
                        Defer
                    </Button>
                </Show>
            </Show>
        </div>
    );
}

function DecisionPanelFull(props: DecisionViewProps): JSX.Element {
    const s = props.s;
    return (
        <div
            ref={(el) => {
                s.rootEl = el;
            }}
            class="agent-decision-panel"
            classList={{
                "agent-decision-panel--high-risk": s.risk() === "high",
                "agent-decision-panel--armed": s.highRiskArmed(),
            }}
            role="dialog"
            aria-label="Permission decision required"
            tabIndex={-1}
        >
            <DecisionPanelClip getEl={() => s.rootEl} />
            <DecisionHeader {...props} />
            <DecisionDetails {...props} />
            <ScopeFieldset {...props} />
            <Show when={s.denyMode()}>
                <DenyFeedback {...props} />
            </Show>
            <DecisionActionsRow {...props} />
        </div>
    );
}

/** The panel for the head request: expanded, or the minimized bar. */
export function DecisionView(props: DecisionViewProps): JSX.Element {
    return (
        <Show when={props.s.request()}>
            <Show when={!props.s.minimized()} fallback={<MinimizedBar {...props} />}>
                <DecisionPanelFull {...props} />
            </Show>
        </Show>
    );
}
