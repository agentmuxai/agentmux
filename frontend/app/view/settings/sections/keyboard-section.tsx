// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings → Keyboard shortcuts — docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §4. Every command with
// its keys now and its default, searchable, with Reset per command and for all. The person's keys are the
// `keybindings` setting, written as a diff from the defaults (keybindings/edit.ts); Help, the palette and the
// tooltips follow it at once (§6).

import { createEffect, createMemo, on, Show, type JSX } from "solid-js";

import { keyPlatform } from "@/app/keybindings";
import { bindingsView, keybindingIssues } from "@/app/keybindings/registry";
import { settingsAtom } from "@/app/store/global";
import { set, SettingRow } from "../settings-controls";
import { KEYBOARD_SETTINGS } from "./keyboard-settings-index";
import { categoriesOf, changedCount, createKeyboardFilters, otherEntries, shortcutBlocks } from "./keyboard-section-state";
import { KeyboardToolbar, OtherEntries, ShortcutList, type KeyboardEdit } from "./keyboard-section-parts";
import { EmptyState } from "@/app/element/ui";
import { DURATION_LINGER_MS } from "@/app/motion/tokens";
import { clearKeyboardRowRequest, keyboardRowRequest } from "../keyboard-focus-request";

export { KEYBOARD_SETTINGS } from "./keyboard-settings-index";

// ── Section: Keyboard shortcuts ───────────────────────────────────────────────

export function KeyboardSection(): JSX.Element {
    const platform = keyPlatform();
    const f = createKeyboardFilters();
    let search: HTMLInputElement | undefined;
    // The setting as saved, including entries the registry skipped: an edit keeps them as they are.
    const entries = () => ((settingsAtom() as Record<string, unknown> | undefined)?.["keybindings"] as unknown[] | undefined) ?? [];
    const view = createMemo(() => bindingsView(platform));
    const blocks = createMemo(() => shortcutBlocks(view(), f.filters(), platform));
    const edit: KeyboardEdit = { entries, issues: keybindingIssues, platform, save: (next) => set("keybindings", next) };

    // Help's Customize and Set links: show every command, scroll to the one asked for and mark it for a moment.
    let body: HTMLDivElement | undefined;
    createEffect(
        on(keyboardRowRequest, (request) => {
            if (!request) return;
            clearKeyboardRowRequest();
            f.setQuery("");
            f.setCategory("all");
            f.setChangedOnly(false);
            if (!request.command) return;
            queueMicrotask(() => {
                const line = body?.querySelector<HTMLElement>(`[data-command="${CSS.escape(request.command!)}"]`);
                if (!line) return;
                line.scrollIntoView({ block: "center" });
                line.focus();
                line.classList.add("setting-row--search-highlight");
                setTimeout(() => line.classList.remove("setting-row--search-highlight"), DURATION_LINGER_MS);
            });
        })
    );

    // "/" jumps to the search; the arrow keys walk the lines (each is a stop of its own, not a tab stop).
    const onKeyDown = (e: KeyboardEvent) => {
        const target = e.target as HTMLElement;
        if (e.key === "/" && !(target instanceof HTMLInputElement || target instanceof HTMLSelectElement)) {
            e.preventDefault();
            search?.focus();
            return;
        }
        if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
        const lines = [...(e.currentTarget as HTMLElement).querySelectorAll<HTMLElement>("[data-kb-row]")];
        const at = lines.indexOf(target.closest<HTMLElement>("[data-kb-row]") as HTMLElement);
        if (at < 0 && !(target instanceof HTMLInputElement)) return;
        const next = lines[at < 0 ? 0 : Math.max(0, Math.min(lines.length - 1, at + (e.key === "ArrowDown" ? 1 : -1)))];
        if (!next) return;
        e.preventDefault();
        next.focus();
    };

    return (
        <div ref={body} class="settings-section-body kb-section" onKeyDown={onKeyDown}>
            <SettingRow
                id={KEYBOARD_SETTINGS.shortcuts.id}
                stacked
                label={KEYBOARD_SETTINGS.shortcuts.label}
                description={KEYBOARD_SETTINGS.shortcuts.description}
                control={
                    <KeyboardToolbar
                        f={f}
                        categories={categoriesOf(view())}
                        changed={changedCount(view())}
                        onResetAll={() => set("keybindings", [])}
                        setSearchRef={(el) => (search = el)}
                    />
                }
            />
            <p class="kb-live" role="status" aria-live="polite">
                {blocks().reduce((n, b) => n + b.rows.length, 0)} shown
            </p>
            <Show
                when={blocks().length > 0}
                fallback={<EmptyState title="No shortcut matches">Try another word, or clear the filters.</EmptyState>}
            >
                <ShortcutList blocks={blocks()} edit={edit} />
            </Show>
            <OtherEntries issues={otherEntries(keybindingIssues(), view())} edit={edit} />
        </div>
    );
}
