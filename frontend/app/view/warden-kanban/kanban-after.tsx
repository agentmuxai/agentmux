// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Starts after" in a task's drawer (SPEC_WARDEN_KANBAN_2026_10_20.md §6.15): the tasks it waits for,
// each removable, and a box to add more by number. While one isn't finished, nobody takes the task.

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button, IconButton, TextInput } from "@/app/element/ui";

import { parseAfter } from "./kanban-columns";
import type { KanbanModel } from "./kanban-model";
import type { KanbanAfter, KanbanTask } from "./kanban-types";

const STATE_WORDS: Record<string, string> = {
    open: "waiting",
    held: "on hold",
    claimed: "in progress",
    blocked: "needs you",
    review: "in review",
    done: "done",
    failed: "failed",
    cancelled: "cancelled",
};

function finished(a: KanbanAfter): boolean {
    return a.state === "done" || a.state === "cancelled";
}

export function KanbanAfterSection(props: {
    model: KanbanModel;
    task: KanbanTask;
    onOpen: (id: string) => void;
}): JSX.Element {
    const [adding, setAdding] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    const after = () => props.task.card.after ?? [];
    const open = () => !["done", "cancelled", "failed"].includes(props.task.state);

    const save = async (ids: string[]) => {
        setBusy(true);
        const res = await props.model.act({ action: "edit", id: props.task.id, after: ids });
        setBusy(false);
        if (res) setAdding("");
    };

    return (
        <Show when={after().length > 0 || open()}>
            <section class="kanban-drawer__section">
                <h4>Starts after</h4>
                <Show
                    when={after().length > 0}
                    fallback={<p class="warden-manager-dim">Nothing: it can start as soon as it's taken.</p>}
                >
                    <ul class="kanban-after">
                        <For each={after()}>
                            {(a) => (
                                <li classList={{ "kanban-after--finished": finished(a) }}>
                                    <Button density="compact" tone="quiet" onClick={() => props.onOpen(a.id)}>
                                        #{a.num} {a.title}
                                    </Button>
                                    <span class="warden-manager-dim">{STATE_WORDS[a.state] ?? a.state}</span>
                                    <Show when={open()}>
                                        <IconButton
                                            icon="xmark"
                                            label={`Don't wait for #${a.num}`}
                                            disabled={busy()}
                                            onClick={() =>
                                                void save(
                                                    after()
                                                        .filter((x) => x.id !== a.id)
                                                        .map((x) => x.id)
                                                )
                                            }
                                        />
                                    </Show>
                                </li>
                            )}
                        </For>
                    </ul>
                </Show>
                <Show when={open()}>
                    <form
                        class="kanban-drawer__row"
                        onSubmit={(e) => {
                            e.preventDefault();
                            const more = parseAfter(adding());
                            if (more.length) void save([...after().map((a) => a.id), ...more]);
                        }}
                    >
                        <TextInput
                            density="compact"
                            placeholder="#12"
                            aria-label="Task numbers it starts after"
                            value={adding()}
                            onInput={(e) => setAdding(e.currentTarget.value)}
                        />
                        <Button
                            type="submit"
                            density="compact"
                            icon="plus"
                            busy={busy()}
                            disabled={!parseAfter(adding()).length}
                        >
                            Wait for it
                        </Button>
                    </form>
                </Show>
            </section>
        </Show>
    );
}
