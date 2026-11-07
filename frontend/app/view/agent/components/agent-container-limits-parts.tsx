// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A container agent's limits, the views: a limit's mode and amount with what is in force, what went wrong or was
// saved, and the Close and Save buttons, over the state of agent-container-limits-state.ts.

import { For, Show, type JSX } from "solid-js";

import { Button, Field, Select, TextInput } from "@/app/element/ui";
import type { LimitDraft, LimitMode } from "../container-limits";
import type { ContainerLimitsState } from "./agent-container-limits-state";

const MODES: { value: LimitMode; label: string }[] = [
    { value: "auto", label: "Automatic" },
    { value: "custom", label: "Set a limit" },
    { value: "none", label: "No limit" },
];

/** One limit: its mode, the amount box while it is a set limit, and the notes under it. */
export function LimitRow(props: {
    s: ContainerLimitsState;
    label: string;
    draft: () => LimitDraft;
    set: (d: LimitDraft) => void;
    suffix: string;
    notes: () => string[];
    testid: string;
}): JSX.Element {
    const { s, label, draft, set, suffix, notes, testid } = props;
    return (
        <Field label={label} layout="stacked">
            <div class="agent-container-limits-row">
                <Select
                    value={draft().mode}
                    disabled={s.limits() == null || s.busy()}
                    onChange={(v) => {
                        set({ ...draft(), mode: v as LimitMode });
                        s.setSaved(null);
                    }}
                    data-testid={`${testid}-mode`}
                >
                    <For each={MODES}>{(m) => <option value={m.value}>{m.label}</option>}</For>
                </Select>
                <Show when={draft().mode === "custom"}>
                    <TextInput
                        class="setting-number"
                        inputmode="decimal"
                        value={draft().amount}
                        disabled={s.busy()}
                        onInput={(e) => {
                            set({ ...draft(), amount: e.currentTarget.value });
                            s.setSaved(null);
                        }}
                        data-testid={`${testid}-amount`}
                    />
                    <span>{suffix}</span>
                </Show>
            </div>
            <For each={notes()}>
                {(note) => (
                    <p class="agent-primitive-modal-global-note" data-testid={`${testid}-note`}>
                        {note}
                    </p>
                )}
            </For>
        </Field>
    );
}

/** What is wrong with the drafts, what went wrong with the call (and Retry for a load), and what was saved. */
export function LimitsMessages(props: { s: ContainerLimitsState }): JSX.Element {
    const s = props.s;
    return (
        <>
            <Show when={s.problem()}>
                <div class="agent-primitive-modal-error" role="alert" data-testid="container-limits-problem">
                    {s.problem()}
                </div>
            </Show>
            <Show when={s.error()}>
                <div class="agent-primitive-modal-error" role="alert">
                    {s.error()}
                </div>
            </Show>
            <Show when={s.limits() == null && s.error()}>
                <div class="agent-primitive-modal-actions">
                    <Button onClick={() => void s.load()}>Retry</Button>
                </div>
            </Show>
            <Show when={s.saved()}>
                <p class="agent-primitive-modal-global-note" role="status" data-testid="container-limits-saved">
                    {s.saved()}
                </p>
            </Show>
        </>
    );
}

/** Close (in a modal) and Save. */
export function LimitsActions(props: { s: ContainerLimitsState; onClose?: () => void }): JSX.Element {
    const s = props.s;
    return (
        <div class="agent-primitive-modal-actions">
            <Show when={props.onClose}>{(close) => <Button onClick={() => close()()}>Close</Button>}</Show>
            <Button tone="accent" busy={s.busy()} disabled={!s.dirty() || s.busy()} onClick={() => void s.save()}>
                Save
            </Button>
        </div>
    );
}
