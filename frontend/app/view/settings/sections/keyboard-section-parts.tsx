// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The pieces of Settings → Keyboard shortcuts (keyboard-section.tsx lays them out): the toolbar, a line per
// command, a line per group of commands, and the entries of the setting that could not be used.

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button, FilterInput, Kbd, Pill, Select, Switch } from "@/app/element/ui";
import { formatKey, type KeyPlatform } from "@/app/keybindings/keys";
import type { CommandBindings, KeybindingIssue } from "@/app/keybindings/registry";
import { SectionHeader } from "../settings-controls";
import { KEYBOARD_SETTINGS } from "./keyboard-settings-index";
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
        </div>
    );
}

/** One command: what it does, its keys, whether they are the person's, and the buttons that act on it. */
export function CommandLine(p: { c: CommandBindings; edit: KeyboardEdit; nested?: boolean }): JSX.Element {
    const { c, edit } = p;
    const issues = () => edit.issues().filter((i) => i.command === c.command);
    return (
        <li class="kb-row" classList={{ "kb-row--nested": p.nested, "kb-row--changed": c.changed }} data-kb-row data-command={c.command} tabIndex={-1}>
            <div class="kb-row-main">
                <span class="kb-row-label">{c.label}</span>
                <code class="kb-row-id">{c.command}</code>
                <Show when={c.changed}>
                    <Pill tone="accent">Changed</Pill>
                </Show>
            </div>
            <div class="kb-row-keys">
                <Show when={!c.unbound} fallback={<span class="kb-row-unset">Not set</span>}>
                    <For each={labelsOf(c.keys, edit.platform)}>{(k) => <Kbd>{k}</Kbd>}</For>
                </Show>
                <Show when={c.changed && c.defaults.length > 0}>
                    <span class="kb-row-default">Default: {labelsOf(c.defaults, edit.platform).join(", ")}</span>
                </Show>
            </div>
            <div class="kb-row-actions">
                <Show when={c.changed}>
                    <Button aria-label={`Reset ${c.label}`} onClick={() => edit.save(resetCommand(edit.entries(), c.command, edit.platform))}>
                        Reset
                    </Button>
                </Show>
                <Show when={!c.unbound}>
                    <Button aria-label={`Unbind ${c.label}`} onClick={() => edit.save(unbindCommand(edit.entries(), c.command, edit.platform))}>
                        Unbind
                    </Button>
                </Show>
            </div>
            <Show when={issues().length > 0}>
                <ul class="kb-row-issues" role="list">
                    <For each={issues()}>
                        {(i) => (
                            <li class="kb-row-issue">
                                <span>{i.message}</span>
                                <Button aria-label={`Remove entry ${i.index}`} onClick={() => edit.save(removeEntry(edit.entries(), i.index))}>
                                    Remove entry
                                </Button>
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
        </li>
    );
}

/** Commands that share a help line ("Go to tab 1–8"): one line that opens into its members. */
export function GroupLine(p: { row: ShortcutRow; edit: KeyboardEdit }): JSX.Element {
    const [open, setOpen] = createSignal(false);
    const changed = () => p.row.commands.filter((c) => c.changed).length;
    return (
        <li class="kb-group">
            <div class="kb-row" classList={{ "kb-row--changed": changed() > 0 }} data-kb-row data-group={p.row.id} tabIndex={-1}>
                <div class="kb-row-main">
                    <span class="kb-row-label">{p.row.label}</span>
                    <span class="kb-row-id">{p.row.commands.length} shortcuts</span>
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
            <Show when={open()}>
                <ul class="kb-list" role="list">
                    <For each={p.row.commands}>{(c) => <CommandLine c={c} edit={p.edit} nested />}</For>
                </ul>
            </Show>
        </li>
    );
}

/** The list, a heading per category. */
export function ShortcutList(p: { blocks: CategoryBlock[]; edit: KeyboardEdit }): JSX.Element {
    return (
        <For each={p.blocks}>
            {(block) => (
                <section class="kb-category" aria-label={block.category}>
                    <SectionHeader label={block.category} />
                    <ul class="kb-list" role="list">
                        <For each={block.rows}>
                            {(row) => (
                                <Show when={row.commands.length > 1} fallback={<CommandLine c={row.commands[0]} edit={p.edit} />}>
                                    <GroupLine row={row} edit={p.edit} />
                                </Show>
                            )}
                        </For>
                    </ul>
                </section>
            )}
        </For>
    );
}

/** Entries of the setting that name no command in the list, with a button to remove each. */
export function OtherEntries(p: { issues: KeybindingIssue[]; edit: KeyboardEdit }): JSX.Element {
    return (
        <Show when={p.issues.length > 0}>
            <section class="kb-category" aria-label="Other entries">
                <SectionHeader label="Other entries" />
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
