// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Above the composer, after srv undid a model change the new place refused (model-undone.ts): what was tried and why it
 * was refused, where the agent is again, and a way to try once the conversation is compacted.
 */
import { Show, createSignal, type Accessor, type JSX } from "solid-js";
import { modelUndoneText, type ModelUndone } from "../model-undone";

export interface AgentModelUndoneCardProps {
    card: Accessor<ModelUndone | null>;
    /** Compacts the conversation, then moves the agent to the model it tried. Rejects with the reason it could not. */
    onCompactThenSwitch: (card: ModelUndone) => Promise<void>;
    onDismiss: () => void;
}

export function AgentModelUndoneCard(props: AgentModelUndoneCardProps): JSX.Element {
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const run = async (card: ModelUndone) => {
        setBusy(true);
        setError(null);
        try {
            await props.onCompactThenSwitch(card);
            props.onDismiss();
        } catch (err: any) {
            setError(err?.message ?? String(err));
        } finally {
            setBusy(false);
        }
    };
    return (
        <Show when={props.card()}>
            {(card) => (
                <div class="agent-model-undone" role="status" data-testid="agent-model-undone">
                    <div class="agent-model-undone-text">
                        <span class="agent-model-undone-title">{modelUndoneText(card())}</span>
                        <Show when={card().reason}>
                            <span class="agent-model-undone-reason" title={card().reason}>
                                {card().reason}
                            </span>
                        </Show>
                        <Show when={error()}>
                            <span class="agent-model-undone-error" role="alert">
                                {error()}
                            </span>
                        </Show>
                    </div>
                    <div class="agent-model-undone-actions">
                        <button type="button" class="agent-model-undone-button" disabled={busy()} onClick={() => void run(card())}>
                            {busy() ? "Compacting…" : "Compact, then switch"}
                        </button>
                        <button type="button" class="agent-model-undone-button" disabled={busy()} onClick={() => props.onDismiss()}>
                            Dismiss
                        </button>
                    </div>
                </div>
            )}
        </Show>
    );
}
AgentModelUndoneCard.displayName = "AgentModelUndoneCard";
