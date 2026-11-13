// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Creating and managing boards (SPEC_SHARED_KANBAN_ACROSS_MACHINES_2026_11_01.md §9.2), as a panel in the
// pane, never a modal: the picker's "New board" and "Manage boards…" open it. Every row's moves are the
// ones agents have through `Tasks` (`board_edit`, `board_archive`, …); the operator makes them through
// `kanban.act`. Rename keeps the slug, so refs keep working. Delete is only for an archived board, and a
// board with tasks waits out a visible countdown with a veto.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { Button, IconButton, MenuTrigger, Pill, TextInput, type MenuEntry } from "@/app/element/ui";
import { useTick } from "@/app/hook/useTick";

import { secondsLeft, tierOf } from "./kanban-boards";
import type { KanbanSectionState } from "./kanban-section-state";
import type { KanbanBoardInfo } from "./kanban-types";

/** The agents a board can be limited to: those running now, and any the board already names. */
export function agentChoices(running: readonly string[], named: readonly string[]): string[] {
    return [...new Set([...running, ...named])].sort((a, b) => a.localeCompare(b));
}

/** The menu that limits a board to some agents: "All agents", or a tick on each that sees it. */
export function agentEntries(choices: readonly string[], seen: readonly string[], onChange: (agents: string[]) => void): MenuEntry[] {
    const toggle = (name: string) => onChange(seen.includes(name) ? seen.filter((n) => n !== name) : [...seen, name]);
    return [
        { label: "All agents", type: "radio", checked: seen.length === 0, onSelect: () => onChange([]) },
        { label: "", divider: true },
        ...choices.map((name) => ({ label: name, type: "checkbox" as const, checked: seen.includes(name), onSelect: () => toggle(name) })),
    ];
}

function agentsWords(seen: readonly string[]): string {
    if (seen.length === 0) return "All agents";
    return seen.length <= 2 ? seen.join(", ") : `${seen.length} agents`;
}

function BoardRow(props: { s: KanbanSectionState; b: KanbanBoardInfo; now: number }): JSX.Element {
    const { s, b } = props;
    const [name, setName] = createSignal(b.name);
    const act = (action: Parameters<typeof s.model.act>[0]["action"], more: object = {}) =>
        s.model.act({ action, board: b.slug, ...more });
    const rename = () => {
        const next = name().trim();
        if (!next) return setName(b.name);
        if (next !== b.name) void act("board_edit", { name: next });
    };
    const left = () => secondsLeft(b.pending_delete_at, props.now);
    const choices = () => agentChoices(s.agents().map((a) => a.name), b.agents);
    return (
        <li class="kanban-boards__row" data-testid={`kanban-boards-row-${b.slug}`}>
            <div class="kanban-boards__main">
                <TextInput
                    density="compact"
                    aria-label={`Name of board ${b.slug}`}
                    value={name()}
                    onInput={(e) => setName(e.currentTarget.value)}
                    onBlur={rename}
                    onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                />
                <code class="kanban-boards__slug" title="What a ref says: slug#number">{b.slug}#</code>
                <Show when={b.default}>
                    <Pill tone="accent">default</Pill>
                </Show>
                <Show when={b.archived}>
                    <Pill>archived</Pill>
                </Show>
                <span class="kanban-boards__count" title={tierOf(b.tier).label}>
                    {b.open} open · {b.total} in all
                </span>
            </div>
            <div class="kanban-boards__actions">
                <MenuTrigger items={agentEntries(choices(), b.agents, (agents) => void act("board_edit", { agents }))} aria-label={`Agents that see ${b.name}`}>
                    <Button density="compact" tone="quiet" icon="user-group" title="Which agents see this board">
                        {agentsWords(b.agents)}
                    </Button>
                </MenuTrigger>
                <Show when={!b.default && !b.archived}>
                    <Button density="compact" tone="quiet" onClick={() => void act("board_default")}>
                        Make default
                    </Button>
                </Show>
                <Show when={!b.default}>
                    <Button density="compact" tone="quiet" onClick={() => void act(b.archived ? "board_unarchive" : "board_archive")}>
                        {b.archived ? "Unarchive" : "Archive"}
                    </Button>
                </Show>
                <Show when={b.archived && !b.default && !b.pending_delete_at}>
                    <Button
                        density="compact"
                        tone="danger"
                        icon="trash"
                        title={b.total > 0 ? `Deletes the board and its ${b.total} task(s), after a ${left() || 15} second countdown you can stop` : "Deletes the board"}
                        onClick={() => void act("board_delete")}
                    >
                        Delete
                    </Button>
                </Show>
                <Show when={b.pending_delete_at}>
                    <Button density="compact" tone="accent" icon="rotate-left" onClick={() => void act("board_cancel_delete")}>
                        Keep it ({left()} s)
                    </Button>
                </Show>
            </div>
        </li>
    );
}

function NewBoard(props: { s: KanbanSectionState }): JSX.Element {
    const s = props.s;
    const [name, setName] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    const submit = async (e?: Event) => {
        e?.preventDefault();
        if (!name().trim() || busy()) return;
        setBusy(true);
        const res = await s.model.act({ action: "board_create", name: name().trim() });
        setBusy(false);
        if (!res) return;
        setName("");
        // Show the board that was made.
        const slug = (res as { board?: { slug?: string } }).board?.slug;
        if (slug) s.selectBoard(slug);
    };
    return (
        <form class="kanban-boards__new" onSubmit={(e) => void submit(e)}>
            <TextInput
                density="compact"
                placeholder="New board's name"
                aria-label="New board's name"
                value={name()}
                onInput={(e) => setName(e.currentTarget.value)}
                autofocus={s.managing() === "new"}
            />
            <span class="kanban-boards__share" title="Sharing a board with your other machines comes with sync">
                {tierOf("local").label}
            </span>
            <Button type="submit" density="compact" tone="accent" icon="plus" busy={busy()} disabled={!name().trim()}>
                Create board
            </Button>
        </form>
    );
}

export function KanbanBoardsPanel(props: { s: KanbanSectionState }): JSX.Element {
    const s = props.s;
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));
    return (
        <section class="kanban-boards" aria-label="Boards">
            <div class="kanban-boards__head">
                <strong>Boards</strong>
                <IconButton icon="xmark" label="Close" onClick={() => s.setManaging(false)} />
            </div>
            <ul class="kanban-boards__list">
                <For each={s.boards()}>{(b) => <BoardRow s={s} b={b} now={now()} />}</For>
            </ul>
            <NewBoard s={s} />
        </section>
    );
}
