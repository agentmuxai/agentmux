// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Above the composer, for an agent on a model route that is near or at its daily spend cap (spend-stopped.ts). From 80% a
 * single line says so. At the cap the gateway refuses the agent's requests, and the card says that and offers Raise,
 * Reset today and Dismiss. A route's own cap is raised in Settings, so a stop for it offers only Dismiss.
 */
import { Show, createSignal, type Accessor, type JSX } from "solid-js";
import { Button } from "../../../element/ui/Button";
import { raisedCap, spendStoppedHint, spendStoppedText, spendWarningText, type SpendCard } from "../spend-stopped";

export interface AgentSpendCardProps {
    stopped: Accessor<SpendCard | null>;
    warning: Accessor<SpendCard | null>;
    /** Sets the agent's daily cap. Rejects with the reason it could not. */
    onRaise: (cap: number) => Promise<void>;
    /** Zeroes today's figure. Rejects with the reason it could not. */
    onReset: () => Promise<void>;
    onDismiss: (card: SpendCard) => void;
}

export function AgentSpendCard(props: AgentSpendCardProps): JSX.Element {
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const run = async (work: () => Promise<void>) => {
        setBusy(true);
        setError(null);
        try {
            await work();
        } catch (err: any) {
            setError(err?.message ?? String(err));
        } finally {
            setBusy(false);
        }
    };
    return (
        <>
            <Show when={props.stopped()}>
                {(card) => (
                    <div class="agent-spend-card" role="status" data-testid="agent-spend-stopped">
                        <div class="agent-spend-card-text">
                            <span class="agent-spend-card-title">{spendStoppedText(card())}</span>
                            <span class="agent-spend-card-hint">{spendStoppedHint(card())}</span>
                            <Show when={error()}>
                                <span class="agent-spend-card-error" role="alert">
                                    {error()}
                                </span>
                            </Show>
                        </div>
                        <div class="agent-spend-card-actions">
                            <Show when={card().scope === "agent"}>
                                <Button disabled={busy()} onClick={() => void run(() => props.onRaise(raisedCap(card().cap)))}>
                                    Raise to ${raisedCap(card().cap)}
                                </Button>
                                <Button disabled={busy()} onClick={() => void run(props.onReset)}>
                                    Reset today
                                </Button>
                            </Show>
                            <Button disabled={busy()} onClick={() => props.onDismiss(card())}>
                                Dismiss
                            </Button>
                        </div>
                    </div>
                )}
            </Show>
            <Show when={!props.stopped() && props.warning()}>
                {(card) => (
                    <div class="agent-spend-card agent-spend-card--warning" role="status" data-testid="agent-spend-warning">
                        <span class="agent-spend-card-title">{spendWarningText(card())}</span>
                    </div>
                )}
            </Show>
        </>
    );
}
AgentSpendCard.displayName = "AgentSpendCard";
