// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The board picker (SPEC_SHARED_KANBAN_ACROSS_MACHINES_2026_11_01.md §9.2): a button at the left of the
// Kanban's header with the selected board's name, its tier icon and how many tasks on it wait for the
// operator. Its menu lists the boards (each with its own count and the default marked), then "+ New
// board" and "Manage boards…"; the archived ones sit in a group at the bottom. Creating and managing
// open a panel in the pane (kanban-boards-panel.tsx), never a modal. A task's "Move to board…" lists the
// same boards.

import { Show, type JSX } from "solid-js";

import { Button, MenuTrigger, Pill, type MenuEntry } from "@/app/element/ui";

import { tierOf } from "./kanban-boards";
import type { KanbanSectionState } from "./kanban-section-state";
import type { KanbanBoardInfo } from "./kanban-types";

function needsPill(n: number): JSX.Element | undefined {
    return n > 0 ? (
        <Pill tone="warn" title={`${n} waiting for you on this board`}>
            {n}
        </Pill>
    ) : undefined;
}

/** A board's row in the menu. */
function boardEntry(b: KanbanBoardInfo, selected: string, needs: number, onSelect: () => void): MenuEntry {
    return {
        label: b.name,
        icon: tierOf(b.tier).icon,
        type: "radio",
        checked: b.slug === selected,
        sublabel: b.default ? "default" : undefined,
        trailing: needsPill(needs),
        onSelect,
        testId: `kanban-board-${b.slug}`,
    };
}

/** The menu's rows: the boards, then the board actions, then the archived group. */
export function pickerEntries(s: KanbanSectionState): MenuEntry[] {
    const live = s.boards().filter((b) => !b.archived);
    const archived = s.boards().filter((b) => b.archived);
    const entries: MenuEntry[] = live.map((b) => boardEntry(b, s.selectedSlug(), s.needsOn()[b.slug] ?? 0, () => s.selectBoard(b.slug)));
    entries.push({ label: "", divider: true });
    entries.push({ label: "New board", icon: "plus", onSelect: () => s.setManaging("new"), testId: "kanban-board-new" });
    entries.push({ label: "Manage boards…", icon: "gear", onSelect: () => s.setManaging("manage"), testId: "kanban-board-manage" });
    if (archived.length > 0) {
        entries.push({ label: "", divider: true });
        entries.push({
            label: `Archived (${archived.length})`,
            icon: "box-archive",
            submenu: archived.map((b) => boardEntry(b, s.selectedSlug(), s.needsOn()[b.slug] ?? 0, () => s.selectBoard(b.slug))),
        });
    }
    return entries;
}

/** The rows of a task's "Move to board…": every board it isn't on, but not the archived ones (read-only). */
export function moveEntries(boards: readonly KanbanBoardInfo[], from: string, onMove: (b: KanbanBoardInfo) => void): MenuEntry[] {
    return boards
        .filter((b) => !b.archived && b.slug !== from)
        .map((b) => ({ label: b.name, icon: tierOf(b.tier).icon, sublabel: b.default ? "default" : undefined, onSelect: () => onMove(b) }));
}

export function KanbanBoardPicker(props: { s: KanbanSectionState }): JSX.Element {
    const s = props.s;
    return (
        <Show when={s.boards().length > 0}>
            <MenuTrigger items={pickerEntries(s)} aria-label="Boards" placement="bottom-start" testId="kanban-board-menu">
                <Button
                    density="compact"
                    icon={tierOf(s.selectedInfo()?.tier ?? "local").icon}
                    class="kanban-board-picker"
                    aria-label={`Board: ${s.selectedInfo()?.name ?? ""}`}
                    title={`Board: ${s.selectedInfo()?.name ?? ""} (${tierOf(s.selectedInfo()?.tier ?? "local").label})`}
                    data-testid="kanban-board-picker"
                >
                    <span class="kanban-board-picker__name">{s.selectedInfo()?.name}</span>
                    {needsPill(s.needsOn()[s.selectedSlug()] ?? 0)}
                    <i class="fa-solid fa-chevron-down kanban-board-picker__chevron" aria-hidden="true" />
                </Button>
            </MenuTrigger>
        </Show>
    );
}
