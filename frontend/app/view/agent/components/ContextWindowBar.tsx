// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";

interface ContextWindowBarProps {
    /** Current input-token count (context fill). null = no turn yet. */
    tokens: number | null | undefined;
    /** Model max context window. undefined = provider unknown. */
    contextWindow: number | undefined;
}

function fmtK(n: number): string {
    return `${Math.round(n / 100) / 10}k`;
}

type Band = "low" | "mid" | "high" | "critical";

function band(fraction: number): Band {
    if (fraction >= 0.9) return "critical";
    if (fraction >= 0.75) return "high";
    if (fraction >= 0.5) return "mid";
    return "low";
}

export function ContextWindowBar(props: ContextWindowBarProps): JSX.Element {
    // Reactive accessors so updates to props.tokens mid-turn (successive
    // message_start events) re-render the bar. An IIFE inside <Show> only
    // runs once when `when` first becomes truthy and freezes thereafter.
    const fraction = () =>
        props.tokens != null && props.contextWindow != null
            ? props.tokens / props.contextWindow
            : null;
    const b = () => { const f = fraction(); return f != null ? band(f) : "low"; };
    const pct = () => { const f = fraction(); return f != null ? Math.min(100, Math.round(f * 100)) : 0; };

    return (
        <Show when={props.tokens != null && props.tokens! > 0}>
            <div
                class="ctx-window-bar"
                title={contextTitle(props.tokens!, props.contextWindow)}
                aria-label={contextTitle(props.tokens!, props.contextWindow)}
            >
                <Show
                    when={props.contextWindow != null}
                    fallback={
                        <span class="ctx-window-raw">
                            {fmtK(props.tokens!)} ctx
                        </span>
                    }
                >
                    <div class={`ctx-window-track ctx-window-track--${b()}`}>
                        <div
                            class="ctx-window-fill"
                            style={{ width: `${pct()}%` }}
                        />
                        <span class="ctx-window-label">
                            {fmtK(props.tokens!)} / {fmtK(props.contextWindow!)}
                        </span>
                    </div>
                </Show>
            </div>
        </Show>
    );
}

function contextTitle(tokens: number, contextWindow: number | undefined): string {
    if (contextWindow == null) {
        return `Context: ${tokens.toLocaleString()} tokens`;
    }
    const pct = ((tokens / contextWindow) * 100).toFixed(1);
    return `Context window: ${tokens.toLocaleString()} / ${contextWindow.toLocaleString()} tokens (${pct}%)\nThis is the total conversation history sent to the model on each turn.`;
}

ContextWindowBar.displayName = "ContextWindowBar";
