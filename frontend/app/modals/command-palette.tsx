// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Command palette modal — opened via Ctrl+P, lists all registered commands.
// Uses the canonical `element/modal` primitive, hung from the top of the window with no backdrop; the palette's
// specific chrome (search row + scroll list) lives in the panel body.

import { commandRegistry, isCommandAvailable, type CommandEntry } from "@/app/store/command-registry";
import { recentCommandIds, rememberCommand } from "@/app/store/palette-recent";
import type { ModalCloseProps } from "@/app/store/modalmodel";
import { disableGlobalKeybindings, enableGlobalKeybindings } from "@/app/store/keymodel";
import { isKeyFor } from "@/app/keybindings";
import { SearchIcon } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Modal } from "@/element/modal";
import { searchCommands } from "./command-palette-search";
import { usePaletteAgents } from "./palette-agents";
import { arrangeRecent, paletteRows, paletteSegments, sortCommands, type PaletteRowModel } from "./palette-groups";
import { watchCutLabels } from "./palette-cut-labels";
import { PaletteRow } from "./palette-row";
import { markEnd, markStart } from "@/perf/marks";
import { batch, createEffect, createMemo, createSignal, createUniqueId, For, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import "./command-palette.scss";

/**
 * The row a navigation key selects from row `i` (`last` is the last row), or null for any other key. Up and Down
 * wrap at the ends (R17); a page, Home and End stop there.
 */
function selectionAfterKey(key: string, i: number, last: number, page: number): number | null {
    const to = (row: number) => Math.max(0, Math.min(row, last));
    switch (key) {
        case "ArrowDown":
            return i >= last ? 0 : i + 1;
        case "ArrowUp":
            return i <= 0 ? Math.max(last, 0) : i - 1;
        case "PageDown":
            return to(i + page);
        case "PageUp":
            return to(i - page);
        case "Home":
            return 0;
        case "End":
            return to(last);
        default:
            return null;
    }
}

/** How many rows the list shows at once, for PageUp and PageDown (at least 1). */
function rowsPerPage(list: HTMLElement): number {
    const row = list.querySelector<HTMLElement>(".command-palette-item");
    return row && row.offsetHeight > 0 ? Math.max(1, Math.floor(list.clientHeight / row.offsetHeight) - 1) : 8;
}

/** After a key move: the selected row in view (the first row: the list's top, padding included). */
function keepRowInView(list: HTMLElement, idx: number) {
    if (idx === 0) list.scrollTop = 0;
    else list.querySelector(".command-palette-item.selected")?.scrollIntoView?.({ block: "nearest" });
}

type KeyIntent = { kind: "close" } | { kind: "run"; forceNew: boolean } | { kind: "move"; to: number };

/** What a key does in the palette, or null when it is none of its keys. */
function keyIntent(e: KeyboardEvent, i: number, last: number, list: HTMLElement): KeyIntent | null {
    // While text is being composed (an IME) these keys belong to the composition (R19).
    if (e.isComposing || e.keyCode === 229) return null;
    // The key that opens the palette closes it (R20). The window's own keys are off while it is open.
    if (isKeyFor(e, "view:command-palette")) return { kind: "close" };
    const to = selectionAfterKey(e.key, i, last, rowsPerPage(list));
    if (to != null) return { kind: "move", to };
    return e.key === "Enter" ? { kind: "run", forceNew: e.shiftKey } : null;
}

/** The palette's budgets: a press to the painted palette, and a typed character to its rows (script time). */
const OPEN_BUDGET_MS = 50;
const KEYSTROKE_BUDGET_MS = 16;

/** The measures are in the performance panel and the HUD; one over its budget is also a line in the log. */
function reportOver(name: string, ms: number | null, budget: number): void {
    if (ms != null && ms > budget) console.warn(`[perf] ${name} ${ms.toFixed(1)}ms, over its ${budget}ms budget`);
}

/** From the press that asked for the palette to the frame that shows it: the first frame after this task, which a timer queued from the frame callback follows. */
function markOpenPainted(): void {
    requestAnimationFrame(() => setTimeout(() => reportOver("palette-open", markEnd("palette-open", "painted"), OPEN_BUDGET_MS), 0));
}

/** The script time of `typed`, from the typed character to the rows it leaves (they are in the document when it returns). */
function timeKeystroke(typed: () => void): void {
    markStart("palette-keystroke");
    typed();
    reportOver("palette-keystroke", markEnd("palette-keystroke", "rows"), KEYSTROKE_BUDGET_MS);
}

/** A typed character: the new query, the first row selected and shown, and the rows built again from the first chunk. */
function applyQuery(
    value: string,
    list: HTMLElement,
    state: { setQuery: (q: string) => void; setSelectedIdx: (i: number) => void; restart: () => void }
): void {
    timeKeystroke(() => {
        batch(() => {
            state.setQuery(value);
            state.setSelectedIdx(0);
            state.restart();
        });
        list.scrollTop = 0;
    });
}

/** Runs what the person picked: remembered as recently used (R29), and run after the palette has closed so it doesn't interfere. */
function runPicked(cmd: CommandEntry, forceNew: boolean): void {
    rememberCommand(cmd.id);
    setTimeout(() => void Promise.resolve(cmd.execute({ forceNew })), 0);
}

/** What the palette lists for `query`, and which row is selected (held to the list when it shrinks). */
function usePaletteResults(query: () => string, selectedIdx: () => number) {
    // The agents AgentMux knows, listed after the commands and found by name, seat or state.
    const agents = usePaletteAgents();
    // What can run now, in browsing order. It does not read the query: a keystroke only searches this list and never
    // sorts it again.
    const available = createMemo(() =>
        sortCommands([...commandRegistry.all(), ...agents()].filter((c) => !c.hidden && isCommandAvailable(c)))
    );
    const arranged = createMemo(() => {
        const q = query().trim();
        // Browsing, unfiltered: category+label order. Ranked while actively searching: the exact command first, then
        // commands containing what was typed (category+label order among equals); fuzzy only for a typo. See
        // command-palette-search.ts. Either way the ones the person ran from here lead (R30).
        return arrangeRecent(q ? searchCommands(available(), q) : available(), recentCommandIds(), q !== "");
    });
    const filtered = () => arranged().cmds;
    const clampedIdx = createMemo(() => Math.min(selectedIdx(), Math.max(filtered().length - 1, 0)));
    const rows = createMemo(() => paletteRows(filtered(), query().trim() !== "", arranged().recent));
    // A group's name has room at the right of its first row, and the shortcuts line up beside it.
    const hasGroups = createMemo(() => rows().some((r) => r.group !== undefined));
    return { filtered, clampedIdx, rows, hasGroups };
}

/** How many rows are built at once. The rest follow a frame at a time, so a list of thousands costs one chunk a press. */
export const ROW_CHUNK = 60;

/**
 * The rows built so far: the first `ROW_CHUNK` at once, then another chunk each frame until the list is whole. A keystroke
 * starts over from the first chunk (rows that stay are kept), and a key that moves the selection past the built rows
 * builds through it before it is shown.
 */
function useBuiltRows(rows: () => PaletteRowModel[]) {
    const [built, setBuilt] = createSignal(ROW_CHUNK);
    let frame = 0;
    const grow = (): void => {
        if (frame || built() >= rows().length) return;
        frame = requestAnimationFrame(() => {
            frame = 0;
            setBuilt(built() + ROW_CHUNK);
            grow();
        });
    };
    // The rows past the first chunk, a chunk a frame: from the start, and again whenever the list grows past what is built
    // (an agent starts while the palette is open), not only when a character is typed.
    createEffect(on(() => rows().length, grow));
    onCleanup(() => cancelAnimationFrame(frame));
    return {
        segments: createMemo(() => paletteSegments(rows().slice(0, built()))),
        /** The list changed: build from the first chunk again, and the rest in the frames after. */
        restart: () => {
            setBuilt(ROW_CHUNK);
            grow();
        },
        /** Row `idx` is about to be shown: it, and a chunk after it, are built. */
        buildThrough: (idx: number) => {
            if (idx >= built()) setBuilt(idx + ROW_CHUNK);
        },
    };
}

/** The listbox: the rows in their groups (each group a screen-reader group, the unnamed first one none). */
function PaletteList(props: {
    ref: (el: HTMLDivElement) => void;
    id: string;
    segments: ReturnType<typeof paletteSegments>;
    selectedIdx: number;
    groupColumn: boolean;
    optionId: (i: number) => string;
    onPick: (row: number, shift: boolean) => void;
}): JSX.Element {
    // Groups are told apart by their name and rows by their command's id (both are the same text from one keystroke to
    // the next), so a keystroke adds the rows that now match, removes the ones that no longer do and updates the rest in
    // place. Keyed by the row objects instead, every row would be torn down and built again on every keystroke.
    const groupKeys = createMemo(() => props.segments.map((s) => s.label ?? ""));
    const groupByKey = createMemo(() => new Map(props.segments.map((s) => [s.label ?? "", s])));
    const rowById = createMemo(() => new Map(props.segments.flatMap((s) => s.rows).map((r) => [r.cmd.id, r])));
    let listEl!: HTMLDivElement;
    const cutLabels = { current: null as ReturnType<typeof watchCutLabels> | null };
    onMount(() => (cutLabels.current = watchCutLabels(listEl)));
    onCleanup(() => cutLabels.current?.stop());
    // The rows changed: which labels are cut is read again after the frame that draws them.
    createEffect(() => {
        groupKeys();
        rowById();
        cutLabels.current?.schedule();
    });
    return (
        <div
            class="command-palette-list"
            ref={(el) => {
                listEl = el;
                props.ref(el);
            }}
            id={props.id}
            role="listbox"
            aria-label={t("palette.listLabel")}
        >
            <For each={groupKeys()}>
                {(key) => {
                    const rowIds = createMemo(() => groupByKey().get(key)?.rows.map((r) => r.cmd.id) ?? []);
                    return (
                        <div role={key ? "group" : undefined} aria-label={key || undefined}>
                            <For each={rowIds()}>
                                {(id) => {
                                    // A row that is leaving can be asked once more before it is gone: it keeps its last model.
                                    let last = rowById().get(id)!;
                                    const row = () => (last = rowById().get(id) ?? last);
                                    return (
                                        <PaletteRow
                                            row={row()}
                                            id={props.optionId(row().index)}
                                            selected={row().index === props.selectedIdx}
                                            groupColumn={props.groupColumn}
                                            onPick={(e) => props.onPick(row().index, e.shiftKey)}
                                        />
                                    );
                                }}
                            </For>
                        </div>
                    );
                }}
            </For>
        </div>
    );
}

const CommandPaletteModal = (props: ModalCloseProps): JSX.Element => {
    const [query, setQuery] = createSignal("");
    const [selectedIdx, setSelectedIdx] = createSignal(0);
    let inputRef!: HTMLInputElement;
    let listRef!: HTMLDivElement;
    const { filtered, clampedIdx, rows, hasGroups } = usePaletteResults(query, selectedIdx);
    const { segments, restart, buildThrough } = useBuiltRows(rows);
    const listId = `command-palette-list-${createUniqueId()}`;
    const optionId = (i: number) => `${listId}-option-${i}`;

    onMount(() => {
        disableGlobalKeybindings();
        // Modal-v2 will focus the first focusable after Portal mount
        // (the search input). Keep a defensive explicit focus() in case
        // the order of focusable elements ever changes.
        queueMicrotask(() => inputRef?.focus());
        markOpenPainted();
    });

    onCleanup(() => {
        enableGlobalKeybindings();
    });

    const close = () => props.close();

    function executeSelected(forceNew = false) {
        const cmds = filtered();
        const idx = clampedIdx();
        if (cmds.length === 0) return;
        const cmd = cmds[idx];
        close();
        runPicked(cmd, forceNew);
    }

    // Arrow keys + Enter are palette-specific navigation. ESC and
    // backdrop close are handled by `<Modal>` → `onClose`.
    function handleKeyDown(e: KeyboardEvent) {
        const intent = keyIntent(e, clampedIdx(), filtered().length - 1, listRef);
        if (!intent) return;
        e.preventDefault();
        if (intent.kind === "close") return close();
        if (intent.kind === "run") return executeSelected(intent.forceNew);
        buildThrough(intent.to);
        setSelectedIdx(intent.to);
        keepRowInView(listRef, intent.to);
    }

    function handleInput(e: InputEvent) {
        applyQuery((e.target as HTMLInputElement).value, listRef, { setQuery, setSelectedIdx, restart });
    }

    return (
        <Modal
            open={true}
            onClose={close}
            placement="dropdown"
            backdrop="none"
            panelClass="command-palette-panel"
            ariaLabel="Command palette"
        >
            <div class="command-palette-container" onKeyDown={handleKeyDown}>
                <div class="command-palette-input-row">
                    <SearchIcon />
                    <input
                        ref={inputRef}
                        class="command-palette-input" aria-label="Search commands and agents" data-raw-text-field="the command palette's search line: it owns the arrow, Enter and Escape keys of the list below it, not a form field"
                        type="text"
                        role="combobox"
                        aria-expanded={true}
                        aria-controls={listId}
                        aria-autocomplete="list"
                        aria-activedescendant={filtered().length > 0 ? optionId(clampedIdx()) : undefined}
                        placeholder="Search commands and agents"
                        value={query()}
                        onInput={handleInput}
                        autocomplete="off"
                        spellcheck={false}
                    />
                </div>
                <PaletteList
                    ref={(el) => (listRef = el)}
                    id={listId}
                    segments={segments()}
                    selectedIdx={clampedIdx()}
                    groupColumn={hasGroups()}
                    optionId={optionId}
                    onPick={(row, shift) => {
                        setSelectedIdx(row);
                        executeSelected(shift);
                    }}
                />
                <Show when={filtered().length === 0}>
                    <div class="command-palette-empty" role="status">
                        Nothing matches "{query()}"
                    </div>
                </Show>
            </div>
        </Modal>
    );
};

CommandPaletteModal.displayName = "CommandPaletteModal";

export { CommandPaletteModal };
