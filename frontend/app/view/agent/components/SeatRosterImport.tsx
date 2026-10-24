// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Import roster…" in the Seat tab (docs/specs/SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §3):
 * paste the approved roster's JSON, preview what each line resolves to and
 * which lines don't (with why; nothing is guessed), then apply. The seats are
 * set one by one through `agent.setSeat`, leads first (`orderForApply`), and
 * each line's outcome is shown.
 */

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { seatBadgeText } from "../seat-format";
import { orderForApply, parseRoster, resolveRoster, type RosterAgent, type RosterPlan } from "../seat-roster";

interface SeatRosterImportProps {
    /** This machine's agents (no templates). */
    agents: RosterAgent[];
    onDone: () => void;
}

interface Outcome {
    name: string;
    error: string | null;
}

const EXAMPLE = `{
  "AgentX": { "role": "leader", "qualifier": "product", "lead": null, "area": "agent pane, identity" },
  "Camper": { "role": "worker", "lead": "AgentX", "area": "panes, layout, tabs" }
}`;

export const SeatRosterImport = (props: SeatRosterImportProps): JSX.Element => {
    const [text, setText] = createSignal("");
    const [plan, setPlan] = createSignal<RosterPlan | null>(null);
    const [parseError, setParseError] = createSignal<string | null>(null);
    const [applying, setApplying] = createSignal(false);
    const [outcomes, setOutcomes] = createSignal<Outcome[] | null>(null);

    const preview = () => {
        setOutcomes(null);
        try {
            setPlan(resolveRoster(parseRoster(text()), props.agents));
            setParseError(null);
        } catch (e) {
            setPlan(null);
            setParseError(e instanceof Error ? e.message : String(e));
        }
    };

    const apply = async () => {
        const p = plan();
        if (!p) return;
        setApplying(true);
        const done: Outcome[] = [];
        for (const line of orderForApply(p.seats)) {
            try {
                await RpcApi.AgentSetSeatCommand(TabRpcClient, { agent_id: line.agent.id, seat: line.seat });
                done.push({ name: line.agent.name, error: null });
            } catch (e) {
                done.push({ name: line.agent.name, error: e instanceof Error ? e.message : String(e) });
            }
            setOutcomes([...done]);
        }
        setApplying(false);
    };

    return (
        <div class="agent-seat-roster" data-testid="agent-seat-roster">
            <span class="agent-primitive-modal-field-label">Import roster</span>
            <p class="agent-primitive-modal-global-note">
                A JSON of agent (name or uid) → role, qualifier, lead, area. A lead of null or "operator" means you.
                Names must match exactly.
            </p>
            <textarea
                class="agent-primitive-modal-textarea agent-seat-roster-input"
                aria-label="Roster JSON"
                placeholder={EXAMPLE}
                value={text()}
                disabled={applying()}
                onInput={(e) => {
                    setText(e.currentTarget.value);
                    setPlan(null);
                }}
            />
            <Show when={parseError()}>
                <div class="agent-primitive-modal-error" role="alert">
                    {parseError()}
                </div>
            </Show>
            <Show when={plan()}>
                {(p) => (
                    <div class="agent-seat-roster-preview" data-testid="agent-seat-roster-preview">
                        <Show when={p().seats.length > 0}>
                            <span class="agent-primitive-modal-field-label">Will set</span>
                            <ul class="agent-seat-roster-list">
                                <For each={p().seats}>
                                    {(line) => (
                                        <li>
                                            <strong>{line.agent.name}</strong> — {seatBadgeText({ role: line.seat.role, qualifier: line.seat.qualifier ?? null })}{" "}
                                            — reports to {line.leadName}
                                            <Show when={line.seat.area}> — {line.seat.area}</Show>
                                        </li>
                                    )}
                                </For>
                            </ul>
                        </Show>
                        <Show when={p().unresolved.length > 0}>
                            <span class="agent-primitive-modal-field-label">Not resolved (left alone)</span>
                            <ul class="agent-seat-roster-list" data-testid="agent-seat-roster-unresolved">
                                <For each={p().unresolved}>
                                    {(line) => (
                                        <li>
                                            <strong>{line.ref}</strong>: {line.reason}
                                        </li>
                                    )}
                                </For>
                            </ul>
                        </Show>
                    </div>
                )}
            </Show>
            <Show when={outcomes()}>
                {(list) => (
                    <ul class="agent-seat-roster-list" data-testid="agent-seat-roster-outcomes">
                        <For each={list()}>
                            {(o) => (
                                <li classList={{ "agent-seat-roster-failed": o.error != null }}>
                                    <strong>{o.name}</strong>: {o.error ?? "seat set"}
                                </li>
                            )}
                        </For>
                    </ul>
                )}
            </Show>
            <div class="agent-primitive-modal-actions">
                <Button disabled={applying()} onClick={() => props.onDone()}>
                    {outcomes() ? "Done" : "Cancel"}
                </Button>
                <Show
                    when={plan() && !outcomes()}
                    fallback={
                        <Show when={!outcomes()}>
                            <Button tone="accent" disabled={!text().trim()} onClick={preview}>
                                Preview
                            </Button>
                        </Show>
                    }
                >
                    <Button
                        tone="accent"
                        busy={applying()}
                        disabled={(plan()?.seats.length ?? 0) === 0}
                        onClick={() => void apply()}
                    >
                        Apply {plan()?.seats.length ?? 0} {plan()?.seats.length === 1 ? "seat" : "seats"}
                    </Button>
                </Show>
            </div>
        </div>
    );
};

SeatRosterImport.displayName = "SeatRosterImport";
