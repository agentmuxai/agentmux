// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The operator writes a task down (SPEC_WARDEN_KANBAN_2026_10_20.md §6.4): for an agent or anyone,
// now or on hold.

import { createSignal, type JSX } from "solid-js";

import { Button, Select, Switch, TextInput } from "@/app/element/ui";

import { parseAfter } from "./kanban-columns";
import type { KanbanModel } from "./kanban-model";
import type { KanbanAgent } from "./kanban-types";

export function KanbanNewTask(props: { model: KanbanModel; agents: KanbanAgent[]; onDone: () => void }): JSX.Element {
    const [title, setTitle] = createSignal("");
    const [payload, setPayload] = createSignal("");
    const [agent, setAgent] = createSignal("");
    const [hold, setHold] = createSignal(false);
    // A feature starts as a spec, then its build (the stages spec §5.1); untick for a small fix.
    const [specFirst, setSpecFirst] = createSignal(true);
    const [after, setAfter] = createSignal("");
    const [busy, setBusy] = createSignal(false);

    const submit = async (e?: Event) => {
        e?.preventDefault();
        if (!title().trim() || busy()) return;
        setBusy(true);
        const res = await props.model.act({
            action: "create",
            title: title().trim(),
            payload: payload().trim() || undefined,
            agent: agent() || undefined,
            hold: hold() || undefined,
            after: parseAfter(after()).length ? parseAfter(after()) : undefined,
            first: specFirst() ? "spec" : undefined,
        });
        setBusy(false);
        if (res) props.onDone();
    };

    return (
        <form class="kanban-new" onSubmit={(e) => void submit(e)}>
            <TextInput
                density="compact"
                placeholder="What needs doing"
                aria-label="Task title"
                value={title()}
                onInput={(e) => setTitle(e.currentTarget.value)}
                autofocus
            />
            <textarea
                class="kanban-drawer__text"
                rows={4}
                placeholder="The full instruction, as a prompt the agent can act on alone (optional: the title is used when empty)"
                aria-label="Task prompt"
                value={payload()}
                onInput={(e) => setPayload(e.currentTarget.value)}
            />
            <div class="kanban-drawer__row">
                <TextInput
                    density="compact"
                    placeholder="Starts after (#12)"
                    aria-label="Task numbers it starts after"
                    value={after()}
                    onInput={(e) => setAfter(e.currentTarget.value)}
                />
                <Select
                    density="compact"
                    ariaLabel="Who it is for"
                    value={agent()}
                    onChange={setAgent}
                    options={[
                        { value: "", label: "For anyone" },
                        ...props.agents.map((a) => ({ value: a.name, label: `For ${a.name}` })),
                    ]}
                />
                <label
                    class="kanban-new__hold"
                    title="Files a spec, and a build that starts after it once the spec is done. Untick for a small fix."
                >
                    <Switch checked={specFirst()} onChange={setSpecFirst} density="compact" ariaLabel="Spec first" />
                    Spec first
                </label>
                <label class="kanban-new__hold">
                    <Switch checked={hold()} onChange={setHold} density="compact" ariaLabel="On hold" />
                    On hold
                </label>
                <Button
                    type="submit"
                    density="compact"
                    tone="accent"
                    icon="plus"
                    busy={busy()}
                    disabled={!title().trim()}
                >
                    Add task
                </Button>
                <Button density="compact" tone="quiet" onClick={props.onDone}>
                    Cancel
                </Button>
            </div>
        </form>
    );
}
