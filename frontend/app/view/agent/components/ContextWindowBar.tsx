// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import { compactionThreshold } from "@/app/store/agent-pane-state/context-window";

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
    // Bar FILL is fraction of the real window (clamped — never implies >100%).
    // The COLOUR BAND tracks proximity to the auto-compaction threshold (~33K
    // below the window), so "critical" means "about to compact" rather than
    // "at 100% of the window".
    const fillPct = () =>
        props.tokens != null && props.contextWindow != null
            ? Math.min(100, Math.round((props.tokens / props.contextWindow) * 100))
            : 0;
    const b = () =>
        props.tokens != null && props.contextWindow != null
            ? band(props.tokens / compactionThreshold(props.contextWindow))
            : "low";

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
                            style={{ width: `${fillPct()}%` }}
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
    return `Context window: ${tokens.toLocaleString()} / ${contextWindow.toLocaleString()} tokens (${pct}%)\nThis is the total conversation history sent to the model on each turn.\nAuto-compacts around ${compactionThreshold(contextWindow).toLocaleString()} tokens.`;
}

ContextWindowBar.displayName = "ContextWindowBar";
