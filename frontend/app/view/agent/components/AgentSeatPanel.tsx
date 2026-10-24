// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSeatPanel — the "Seat" tab inside AgentStashModal, also opened from
 * My Agents' row menu ("Seat…"): the agent's role, qualifier, lead and area,
 * set only by the operator through `agent.setSeat`
 * (docs/specs/SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §3). Also hosts
 * "Import roster…" (SeatRosterImport).
 *
 * The seat shown is the one `listagents` gives: the stored seat, or a spawned
 * worker's effective one (`set_at_ms === 0`), which the tab marks "from
 * spawn" so the operator can give it a real one.
 */

import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

import { Button, Field, Select, TextInput, type SelectOption } from "@/app/element/ui";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi, type AgentDefinition } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentSeat } from "@/types/rpc/AgentSeat";
import { MAX_AREA_CHARS, MAX_QUALIFIER_CHARS, SEAT_ROLES, type SeatRole } from "../seat-format";
import { SeatRosterImport } from "./SeatRosterImport";
import "./AgentPrimitiveModal.scss";
import "./AgentSeatPanel.scss";

/** Shown when srv refuses a channel-local agent (§2): it has no shared definition to hold a seat. */
export const NO_SHARED_DEFINITION_NOTE =
    "This agent exists only in this channel, with no shared definition, so it can't have a seat.";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

interface AgentSeatPanelProps {
    agentId: string;
    agentName: string;
    /** Given when hosted in a modal (My Agents' "Seat…"): shows a Close button. */
    onClose?: () => void;
}

/** This machine's agents (`listagents`), refetched on `agents:changed`. */
function useAgentList(): [() => AgentDefinition[] | undefined, () => string | null] {
    const [agents, setAgents] = createSignal<AgentDefinition[] | undefined>(undefined);
    const [error, setError] = createSignal<string | null>(null);
    onMount(() => {
        let cancelled = false;
        const load = async () => {
            try {
                const list = await RpcApi.ListAgentDefinitionsCommand(TabRpcClient);
                if (cancelled) return;
                setAgents(list ?? []);
                setError(null);
            } catch (e) {
                if (!cancelled) setError(errorText(e));
            }
        };
        void load();
        const unsub = onMuxEvent(WpsEvent.AgentsChanged, () => void load());
        onCleanup(() => {
            cancelled = true;
            unsub();
        });
    });
    return [agents, error];
}

export const AgentSeatPanel = (props: AgentSeatPanelProps): JSX.Element => {
    const [agents, loadError] = useAgentList();
    const self = createMemo(() => agents()?.find((a) => a.id === props.agentId));
    /** Agents only, not templates: what a lead or a roster line can name. */
    const people = createMemo(() => (agents() ?? []).filter((a) => a.is_seeded === 0));
    const seat = createMemo<AgentSeat | undefined>(() => self()?.seat);

    const [role, setRole] = createSignal<SeatRole | "">("");
    const [qualifier, setQualifier] = createSignal("");
    const [lead, setLead] = createSignal("");
    const [area, setArea] = createSignal("");
    const [busy, setBusy] = createSignal<"save" | "clear" | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [saved, setSaved] = createSignal<string | null>(null);
    const [importing, setImporting] = createSignal(false);

    // Fill the form from the seat as listed, and again whenever that seat
    // changes (a save, a clear, a roster import, another window).
    let filledFrom: string | undefined;
    createEffect(() => {
        if (agents() === undefined) return;
        const s = seat();
        const key = JSON.stringify(s ?? null);
        if (key === filledFrom) return;
        filledFrom = key;
        setRole(s?.role ?? "");
        setQualifier(s?.qualifier ?? "");
        setLead(s?.lead ?? "");
        setArea(s?.area ?? "");
    });

    const roleOptions = (): SelectOption[] => [
        { value: "", label: "Choose a role…", disabled: true },
        ...SEAT_ROLES.map((r) => ({ value: r.value, label: r.label })),
    ];
    const leadOptions = createMemo((): SelectOption[] => {
        const options = people()
            .filter((a) => a.id !== props.agentId)
            .map((a) => ({ value: a.id, label: a.name }))
            .sort((a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: "base" }));
        const current = lead();
        if (current && !options.some((o) => o.value === current)) {
            options.unshift({ value: current, label: "An agent not on this machine" });
        }
        return [{ value: "", label: "The operator" }, ...options];
    });

    const run = async (kind: "save" | "clear"): Promise<void> => {
        setError(null);
        setSaved(null);
        setBusy(kind);
        try {
            const r = role();
            const input =
                kind === "clear" || r === ""
                    ? null
                    : { role: r, qualifier: qualifier().trim(), lead: lead(), area: area().trim() };
            await RpcApi.AgentSetSeatCommand(TabRpcClient, { agent_id: props.agentId, seat: input });
            setSaved(kind === "save" ? "Seat saved." : "Seat cleared.");
        } catch (e) {
            const text = errorText(e);
            setError(/no shared definition/.test(text) ? NO_SHARED_DEFINITION_NOTE : text);
        } finally {
            setBusy(null);
        }
    };

    return (
        <div class="agent-primitive-modal-detail agent-seat-panel" data-testid="agent-seat-panel">
            <div class="agent-primitive-modal-readonly">
                <p class="agent-primitive-modal-global-note">
                    {props.agentName}'s seat: its role, whom it reports to and what it owns. Only you set it; agents
                    read it.
                </p>
                <Show when={loadError()}>
                    <div class="agent-primitive-modal-error">{loadError()}</div>
                </Show>
                <Show when={agents() !== undefined && !self()}>
                    <p class="agent-primitive-modal-global-note">This pane has no saved agent, so it has no seat.</p>
                </Show>
                <Show when={self()}>
                    <Show when={seat()?.set_at_ms === 0}>
                        <p class="agent-seat-panel-from-spawn" data-testid="agent-seat-from-spawn">
                            From spawn: it reports to the agent that spawned it. Save to give it a seat of its own.
                        </p>
                    </Show>
                    <Show when={!seat()}>
                        <p class="agent-primitive-modal-global-note">No seat yet.</p>
                    </Show>
                    <div class="agent-seat-panel-form">
                        <Field label="Role" layout="stacked">
                            <Select options={roleOptions()} value={role()} onChange={(v) => setRole(v as SeatRole)} />
                        </Field>
                        <Field label="Qualifier" layout="stacked" hint='Shown after the role, e.g. "product".'>
                            <TextInput
                                value={qualifier()}
                                maxLength={MAX_QUALIFIER_CHARS}
                                placeholder="product"
                                onInput={(e) => setQualifier(e.currentTarget.value)}
                            />
                        </Field>
                        <Field label="Reports to" layout="stacked">
                            <Select options={leadOptions()} value={lead()} onChange={setLead} />
                        </Field>
                        <Field label="Area" layout="stacked" hint="One line: what it owns.">
                            <TextInput
                                value={area()}
                                maxLength={MAX_AREA_CHARS}
                                placeholder="panes, layout, tabs"
                                onInput={(e) => setArea(e.currentTarget.value)}
                            />
                        </Field>
                    </div>
                    <Show when={error()}>
                        <div class="agent-primitive-modal-error" role="alert">
                            {error()}
                        </div>
                    </Show>
                    <Show when={saved()}>
                        <p class="agent-primitive-modal-global-note" role="status">
                            {saved()}
                        </p>
                    </Show>
                    <div class="agent-primitive-modal-actions">
                        <Button
                            tone="danger"
                            busy={busy() === "clear"}
                            disabled={busy() !== null || !seat()}
                            onClick={() => void run("clear")}
                        >
                            Clear
                        </Button>
                        <Button
                            tone="accent"
                            busy={busy() === "save"}
                            disabled={busy() !== null || role() === ""}
                            onClick={() => void run("save")}
                        >
                            Save
                        </Button>
                    </div>
                </Show>
                <div class="agent-seat-panel-roster">
                    <Show
                        when={importing()}
                        fallback={
                            <Button icon="file-import" onClick={() => setImporting(true)}>
                                Import roster…
                            </Button>
                        }
                    >
                        <SeatRosterImport
                            agents={people().map((a) => ({ id: a.id, name: a.name }))}
                            onDone={() => setImporting(false)}
                        />
                    </Show>
                </div>
                <Show when={props.onClose}>
                    {(close) => (
                        <div class="agent-primitive-modal-actions">
                            <Button onClick={() => close()()}>Close</Button>
                        </div>
                    )}
                </Show>
            </div>
        </div>
    );
};

AgentSeatPanel.displayName = "AgentSeatPanel";
