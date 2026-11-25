// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Import roster…" in the Seat tab (docs/specs/SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §3):
 * paste the approved roster's JSON, preview what each line resolves to and
 * which lines don't (with why; nothing is guessed), then apply. The seats are
 * set one by one through `agent.setSeat`, leads first (`orderForApply`), a
 * line refused as a cycle is retried once others land (`applyInPasses`), and
 * each line's outcome is shown.
 */

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button, TextArea } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { seatBadgeText } from "../../../agent-model/seat-format";
import {
    applyInPasses,
    orderForApply,
    parseRoster,
    resolveRoster,
    type ApplyOutcome,
    type RosterAgent,
    type RosterPlan,
} from "../seat-roster";

interface SeatRosterImportProps {
    /** This machine's agents (no templates). */
    agents: RosterAgent[];
    onDone: () => void;
}

interface Outcome {
    name: string;
    error: string | null;
}

/** What one "Will set" line says: who, the seat's badge and whom it reports to. */
const lineParams = (line: RosterPlan["seats"][number]) => ({
    name: line.agent.name,
    badge: seatBadgeText({ role: line.seat.role, qualifier: line.seat.qualifier ?? null }),
    lead: line.leadName,
});

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
        const show = (results: ApplyOutcome[]) => setOutcomes(results.map((r) => ({ name: r.seat.agent.name, error: r.error })));
        show(
            await applyInPasses(
                orderForApply(p.seats),
                async (line) => {
                    await RpcApi.AgentSetSeatCommand(TabRpcClient, { agent_id: line.agent.id, seat: line.seat });
                },
                show
            )
        );
        setApplying(false);
    };

    return (
        <div class="agent-seat-roster" data-testid="agent-seat-roster">
            <span class="agent-primitive-modal-field-label">{t("seatRoster.title")}</span>
            <p class="agent-primitive-modal-global-note">
                {t("seatRoster.help")}
            </p>
            <TextArea variant="plain"
                class="agent-primitive-modal-textarea agent-seat-roster-input"
                aria-label={t("seatRoster.jsonLabel")}
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
                            <span class="agent-primitive-modal-field-label">{t("seatRoster.willSet")}</span>
                            <ul class="agent-seat-roster-list">
                                <For each={p().seats}>
                                    {(line) => (
                                        <li>
                                            <Rich k="seatRoster.line" params={lineParams(line)} />
                                            <Show when={line.seat.area}> — {line.seat.area}</Show>
                                        </li>
                                    )}
                                </For>
                            </ul>
                        </Show>
                        <Show when={p().unresolved.length > 0}>
                            <span class="agent-primitive-modal-field-label">{t("seatRoster.unresolved")}</span>
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
                                    <strong>{o.name}</strong>: {o.error ?? t("seatRoster.seatSet")}
                                </li>
                            )}
                        </For>
                    </ul>
                )}
            </Show>
            <div class="agent-primitive-modal-actions">
                <Button disabled={applying()} onClick={() => props.onDone()}>
                    {outcomes() ? t("seatRoster.done") : t("seatRoster.cancel")}
                </Button>
                <Show
                    when={plan() && !outcomes()}
                    fallback={
                        <Show when={!outcomes()}>
                            <Button tone="accent" disabled={!text().trim()} onClick={preview}>
                                {t("seatRoster.preview")}
                            </Button>
                        </Show>
                    }
                >
                    <Button
                        tone="accent"
                        busy={applying()}
                        disabled={(plan()?.seats.length ?? 0) === 0}
                        onClick={() => apply()}
                    >
                        {t("seatRoster.apply", { count: plan()?.seats.length ?? 0 })}
                    </Button>
                </Show>
            </div>
        </div>
    );
};

SeatRosterImport.displayName = "SeatRosterImport";
