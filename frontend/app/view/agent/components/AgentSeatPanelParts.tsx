// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Seat panel's views, over a state from AgentSeatPanelState.ts: the notes, the form and the roster import. */

import { Show, type JSX } from "solid-js";

import { Button, Field, Select, TextInput } from "@/app/element/ui";
import { MAX_AREA_CHARS, MAX_QUALIFIER_CHARS, type SeatRole } from "../../../agent-model/seat-format";
import type { AgentSeatState } from "./AgentSeatPanelState";
import { SeatRosterImport } from "./SeatRosterImport";

export function SeatIntro(props: { s: AgentSeatState; agentName: string }): JSX.Element {
    const s = props.s;
    return (
        <>
            <p class="agent-primitive-modal-global-note">
                {props.agentName}'s seat: its role, whom it reports to and what it owns. Only you set it; agents read
                it.
            </p>
            <Show when={s.loadError()}>
                <div class="agent-primitive-modal-error">{s.loadError()}</div>
            </Show>
            <Show when={s.agents() !== undefined && !s.self()}>
                <p class="agent-primitive-modal-global-note">This pane has no saved agent, so it has no seat.</p>
            </Show>
        </>
    );
}

function SeatFields(props: { s: AgentSeatState }): JSX.Element {
    const s = props.s;
    const f = s.form;
    return (
        <div class="agent-seat-panel-form">
            <Field label="Role" layout="stacked">
                <Select options={s.roleOptions()} value={f.role()} onChange={(v) => f.setRole(v as SeatRole)} />
            </Field>
            <Field label="Qualifier" layout="stacked" hint='Shown after the role, e.g. "product".'>
                <TextInput
                    value={f.qualifier()}
                    maxLength={MAX_QUALIFIER_CHARS}
                    placeholder="product"
                    onInput={(e) => f.setQualifier(e.currentTarget.value)}
                />
            </Field>
            <Field label="Reports to" layout="stacked">
                <Select options={s.leadOptions()} value={f.lead()} onChange={f.setLead} />
            </Field>
            <Field label="Area" layout="stacked" hint="One line: what it owns.">
                <TextInput
                    value={f.area()}
                    maxLength={MAX_AREA_CHARS}
                    placeholder="panes, layout, tabs"
                    onInput={(e) => f.setArea(e.currentTarget.value)}
                />
            </Field>
        </div>
    );
}

function SeatActions(props: { s: AgentSeatState }): JSX.Element {
    const s = props.s;
    const f = s.form;
    return (
        <div class="agent-primitive-modal-actions">
            <Button
                tone="danger"
                busy={f.busy() === "clear"}
                disabled={f.busy() !== null || !s.seat()}
                onClick={() => void s.run("clear")}
            >
                Clear
            </Button>
            <Button
                tone="accent"
                busy={f.busy() === "save"}
                disabled={f.busy() !== null || f.role() === ""}
                onClick={() => void s.run("save")}
            >
                Save
            </Button>
        </div>
    );
}

/** The form for an agent that is listed: the seat's state, the fields, what srv answered, and the buttons. */
export function SeatEditor(props: { s: AgentSeatState }): JSX.Element {
    const s = props.s;
    return (
        <>
            <Show when={s.seat()?.set_at_ms === 0}>
                <p class="agent-seat-panel-from-spawn" data-testid="agent-seat-from-spawn">
                    From spawn: it reports to the agent that spawned it. Save to give it a seat of its own.
                </p>
            </Show>
            <Show when={!s.seat()}>
                <p class="agent-primitive-modal-global-note">No seat yet.</p>
            </Show>
            <SeatFields s={s} />
            <Show when={s.form.error()}>
                <div class="agent-primitive-modal-error" role="alert">
                    {s.form.error()}
                </div>
            </Show>
            <Show when={s.form.saved()}>
                <p class="agent-primitive-modal-global-note" role="status">
                    {s.form.saved()}
                </p>
            </Show>
            <SeatActions s={s} />
        </>
    );
}

export function RosterRow(props: { s: AgentSeatState }): JSX.Element {
    const s = props.s;
    return (
        <div class="agent-seat-panel-roster">
            <Show
                when={s.importing()}
                fallback={
                    <Button icon="file-import" onClick={() => s.setImporting(true)}>
                        Import roster…
                    </Button>
                }
            >
                <SeatRosterImport
                    agents={s.people().map((a) => ({ id: a.id, name: a.name }))}
                    onDone={() => s.setImporting(false)}
                />
            </Show>
        </div>
    );
}
