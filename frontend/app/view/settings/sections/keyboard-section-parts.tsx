// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The pieces of Settings → Keyboard shortcuts (keyboard-section.tsx lays them out): the toolbar, a line per
// command, a line per group of commands, and the entries of the setting that could not be used.

import { t } from "@/app/i18n/i18n";
import { createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { Button, FilterInput, Kbd, Pill, Select, Switch } from "@/app/element/ui";
import { formatKey, type KeyPlatform } from "@/app/keybindings/keys";
import type { CommandBindings, KeybindingIssue } from "@/app/keybindings/registry";
import { SectionHeader } from "../settings-controls";
import { KeySearch } from "./keyboard-key-search";
import { KeyRecorder } from "./keyboard-recorder";
import { KEYBOARD_SETTINGS } from "./keyboard-settings-index";
import { KeyboardTransfer } from "./keyboard-transfer";
import { removeEntry, resetCommand, unbindCommand, type CategoryBlock, type CategoryFilter, type KeyboardFilters, type ShortcutRow } from "./keyboard-section-state";
import type { KeyCategory } from "@/app/keybindings/defaults";

/** What every line needs to change the setting: its entries as saved, this platform, and where the new ones go. */
export interface KeyboardEdit {
    entries: () => unknown[];
    /** The entries the registry skipped, with their place in the setting. */
    issues: () => KeybindingIssue[];
    platform: KeyPlatform;
    save: (next: unknown[]) => void;
}

const labelsOf = (keys: string[], platform: KeyPlatform): string[] => keys.map((k) => formatKey(k, platform));

/** The search, the category, "changed only" and Reset all. */
export function KeyboardToolbar(p: {
    f: KeyboardFilters;
    categories: KeyCategory[];
    changed: number;
    edit: KeyboardEdit;
    onResetAll: () => void;
    setSearchRef: (el: HTMLInputElement) => void;
}): JSX.Element {
    const [confirming, setConfirming] = createSignal(false);
    return (
        <div class="kb-toolbar" role="search">
            <FilterInput
                value={p.f.query()}
                onInput={p.f.setQuery}
                placeholder="Search by action or key"
                ariaLabel="Search shortcuts by action or key"
                inputRef={p.setSearchRef}
                testId="kb-search"
            />
            <KeySearch f={p.f} platform={p.edit.platform} />
            <Select
                class="setting-select"
                ariaLabel="Category"
                value={p.f.category()}
                onChange={(v) => p.f.setCategory(v as CategoryFilter)}
                options={[{ value: "all", label: "All categories" }, ...p.categories.map((c) => ({ value: c, label: c }))]}
            />
            <label class="kb-toolbar-switch">
                <Switch checked={p.f.changedOnly()} onChange={p.f.setChangedOnly} ariaLabel="Changed only" />
                <span>Changed only</span>
            </label>
            <Button
                id={`setting-${KEYBOARD_SETTINGS.resetAll.id}`}
                tone={confirming() ? "danger" : undefined}
                disabled={p.changed === 0 && !confirming()}
                onClick={() => {
                    if (!confirming()) return setConfirming(true);
                    setConfirming(false);
                    p.onResetAll();
                }}
                onBlur={() => setConfirming(false)}
            >
                {confirming() ? "Click again to reset all" : "Reset all"}
            </Button>
            <KeyboardTransfer edit={p.edit} />
        </div>
    );
}

/** The latest value of something looked up by id, kept while its line is being removed. */
const latest = <T,>(get: () => T | undefined): (() => T) => createMemo<T>((prev) => get() ?? prev);

/** The skipped entries of the setting that name a command on this line, each with a button to remove it. */
function IssueList(p: { issues: KeybindingIssue[]; edit: KeyboardEdit }): JSX.Element {
    return (
        <Show when={p.issues.length > 0}>
            <ul class="kb-row-issues" role="list">
                <For each={p.issues}>
                    {(i) => (
                        <li class="kb-row-issue">
                            <span>{i.message}</span>
                            <Button aria-label={`Remove entry ${i.index}`} onClick={() => p.edit.save(removeEntry(p.edit.entries(), i.index))}>
                                Remove entry
                            </Button>
                        </li>
                    )}
                </For>
            </ul>
        </Show>
    );
}

/** One command: what it does, its keys, whether they are the person's, and the buttons that act on it. */
export function CommandLine(p: { c: () => CommandBindings; edit: KeyboardEdit; nested?: boolean }): JSX.Element {
    const { c, edit } = p;
    let rowEl: HTMLLIElement | undefined;
    const [recording, setRecording] = createSignal(false);
    // The line stays mounted while its keys change, but the button that was pressed may go: the line keeps the focus.
    const act = (next: unknown[]) => {
        edit.save(next);
        setRecording(false);
        rowEl?.focus();
    };
    return (
        <li
            ref={rowEl}
            class="kb-row"
            classList={{ "kb-row--nested": p.nested, "kb-row--changed": c().changed }}
            data-kb-row
            data-command={c().command}
            tabIndex={-1}
            onKeyDown={(e) => {
                // Enter on the line itself starts recording; Enter on a button inside it is that button's.
                if (e.key !== "Enter" || e.target !== e.currentTarget) return;
                e.preventDefault();
                setRecording(true);
            }}
        >
            <div class="kb-row-main">
                <span class="kb-row-label">{c().label}</span>
                <code class="kb-row-id">{c().command}</code>
                <Show when={c().changed}>
                    <Pill tone="accent">Changed</Pill>
                </Show>
            </div>
            <div class="kb-row-keys">
                <Show when={!c().unbound} fallback={<span class="kb-row-unset">Not set</span>}>
                    <For each={labelsOf(c().keys, edit.platform)}>{(k) => <Kbd>{k}</Kbd>}</For>
                </Show>
                <Show when={c().changed && c().defaults.length > 0}>
                    <span class="kb-row-default">Default: {labelsOf(c().defaults, edit.platform).join(", ")}</span>
                </Show>
            </div>
            <div class="kb-row-actions">
                <Button aria-label={`${c().unbound ? "Set key for" : "Edit keys of"} ${c().label}`} aria-expanded={recording()} onClick={() => setRecording(!recording())}>
                    {c().unbound ? "Set key" : "Edit"}
                </Button>
                <Show when={c().changed}>
                    <Button aria-label={`Reset ${c().label}`} onClick={() => act(resetCommand(edit.entries(), c().command, edit.platform))}>
                        Reset
                    </Button>
                </Show>
                <Show when={!c().unbound}>
                    <Button aria-label={`Unbind ${c().label}`} onClick={() => act(unbindCommand(edit.entries(), c().command, edit.platform))}>
                        Unbind
                    </Button>
                </Show>
            </div>
            <Show when={recording()}>
                <KeyRecorder c={c} edit={edit} onSave={act} onClose={() => { setRecording(false); rowEl?.focus(); }} />
            </Show>
            <Show when={!p.nested}>
                <IssueList issues={edit.issues().filter((i) => i.command === c().command)} edit={edit} />
            </Show>
        </li>
    );
}

/** Commands that share a help line ("Go to tab 1–8"): one line that opens into its members. */
export function GroupLine(p: { row: () => ShortcutRow; edit: KeyboardEdit }): JSX.Element {
    const [open, setOpen] = createSignal(false);
    const changed = () => p.row().commands.filter((c) => c.changed).length;
    const commandIds = createMemo(() => p.row().commands.map((c) => c.command));
    // A member's skipped entries show on the group line, so they can be removed while it is closed.
    const issues = () => p.edit.issues().filter((i) => commandIds().includes(i.command));
    return (
        <li class="kb-group">
            <div class="kb-row" classList={{ "kb-row--changed": changed() > 0 }} data-kb-row data-group={p.row().id} tabIndex={-1}>
                <div class="kb-row-main">
                    <span class="kb-row-label">{p.row().label}</span>
                    <span class="kb-row-id">{p.row().commands.length} shortcuts</span>
                    <Show when={changed() > 0}>
                        <Pill tone="accent">{changed()} changed</Pill>
                    </Show>
                </div>
                <div class="kb-row-actions">
                    <Button aria-expanded={open()} onClick={() => setOpen(!open())}>
                        {open() ? "Hide" : "Show"}
                    </Button>
                </div>
            </div>
            <IssueList issues={issues()} edit={p.edit} />
            <Show when={open()}>
                <ul class="kb-list" role="list">
                    <For each={commandIds()}>
                        {(id) => <CommandLine c={latest(() => p.row().commands.find((c) => c.command === id))} edit={p.edit} nested />}
                    </For>
                </ul>
            </Show>
        </li>
    );
}

/** A line of the list: a command, or a group of them. Looked up by id so it stays mounted as the list changes. */
function RowLine(p: { id: string; rowById: () => Map<string, ShortcutRow>; edit: KeyboardEdit }): JSX.Element {
    const row = latest(() => p.rowById().get(p.id));
    return (
        <Show when={row().commands.length > 1} fallback={<CommandLine c={() => row().commands[0]} edit={p.edit} />}>
            <GroupLine row={row} edit={p.edit} />
        </Show>
    );
}

/** One category: a heading and its lines. */
function CategoryList(p: { category: KeyCategory; blockOf: () => Map<KeyCategory, CategoryBlock>; edit: KeyboardEdit }): JSX.Element {
    const block = latest(() => p.blockOf().get(p.category));
    const rowIds = createMemo(() => block().rows.map((r) => r.id));
    const rowById = createMemo(() => new Map(block().rows.map((r) => [r.id, r])));
    return (
        <section class="kb-category" aria-label={p.category}>
            <SectionHeader label={p.category} />
            <ul class="kb-list" role="list">
                <For each={rowIds()}>{(id) => <RowLine id={id} rowById={rowById} edit={p.edit} />}</For>
            </ul>
        </section>
    );
}

/**
 * The list, a heading per category. Categories, lines and members are walked by id, never by object: every change
 * of the setting builds new objects, and a line walked by object would be torn down, closing its group and
 * dropping the focus.
 */
export function ShortcutList(p: { blocks: CategoryBlock[]; edit: KeyboardEdit }): JSX.Element {
    const categories = createMemo(() => p.blocks.map((b) => b.category));
    const blockOf = createMemo(() => new Map(p.blocks.map((b) => [b.category, b])));
    return <For each={categories()}>{(category) => <CategoryList category={category} blockOf={blockOf} edit={p.edit} />}</For>;
}

/** Entries of the setting that name no command in the list, with a button to remove each. */
export function OtherEntries(p: { issues: KeybindingIssue[]; edit: KeyboardEdit }): JSX.Element {
    return (
        <Show when={p.issues.length > 0}>
            <section class="kb-category" aria-label="Other entries">
                <SectionHeader label={t("settings.keyboard.group.otherEntries")} />
                <ul class="kb-list" role="list">
                    <For each={p.issues}>
                        {(i) => (
                            <li class="kb-row kb-row--issue" data-kb-row tabIndex={-1}>
                                <div class="kb-row-main">
                                    <code class="kb-row-id">keybindings[{i.index}]</code>
                                    <span class="kb-row-issue">{i.message}</span>
                                </div>
                                <div class="kb-row-actions">
                                    <Button aria-label={`Remove entry ${i.index}`} onClick={() => p.edit.save(removeEntry(p.edit.entries(), i.index))}>
                                        Remove entry
                                    </Button>
                                </div>
                            </li>
                        )}
                    </For>
                </ul>
            </section>
        </Show>
    );
}
