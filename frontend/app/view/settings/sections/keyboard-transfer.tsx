// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Import and export in Settings → Keyboard shortcuts (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §9):
// Export copies the `keybindings` array; Import takes pasted text, says what it would change, and merges it or
// replaces the setting with it. The rules are in keyboard-transfer-state.ts.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { Button, TextArea } from "@/app/element/ui";
import { writeText } from "@/util/clipboard";
import type { KeyboardEdit } from "./keyboard-section-parts";
import { exportText, mergeImport, parseImport, previewImport, previewText } from "./keyboard-transfer-state";

/** The pasted text and what importing it would do, with the buttons that do it. */
function ImportPanel(p: { edit: KeyboardEdit; onDone: (note: string) => void; onCancel: () => void }): JSX.Element {
    const [text, setText] = createSignal("");
    const [confirming, setConfirming] = createSignal(false);
    const parsed = createMemo(() => (text().trim() ? parseImport(text()) : null));
    const preview = createMemo(() => {
        const r = parsed();
        return r && "entries" in r ? previewImport(p.edit.entries(), r.entries) : null;
    });
    const imported = (usable: unknown[], next: unknown[]) => {
        p.edit.save(next);
        p.onDone(`Imported ${usable.length} ${usable.length === 1 ? "entry" : "entries"}.`);
    };
    return (
        <div class="kb-import" role="group" aria-label="Import keybindings">
            <TextArea
                mono
                rows={6}
                aria-label="Keybindings to import"
                placeholder="Paste the exported keybindings (JSON)"
                value={text()}
                onInput={(e) => {
                    setConfirming(false);
                    setText(e.currentTarget.value);
                }}
            />
            <p class="kb-import-preview" role="status" aria-live="polite">
                <Show when={parsed()}>{(parsed() as { error?: string }).error ?? (preview() ? previewText(preview()!) : "")}</Show>
            </p>
            <Show when={preview() && preview()!.unusable.length > 0}>
                <ul class="kb-import-unusable" role="list">
                    <For each={preview()!.unusable.slice(0, 5)}>{(u) => <li>Entry {u.index + 1}: {u.message}</li>}</For>
                </ul>
            </Show>
            <div class="kb-import-actions">
                <Button tone="accent" disabled={!preview() || preview()!.usable.length === 0} onClick={() => imported(preview()!.usable, mergeImport(p.edit.entries(), preview()!.usable))}>
                    Merge
                </Button>
                <Button
                    tone={confirming() ? "danger" : undefined}
                    disabled={!preview() || preview()!.usable.length === 0}
                    onClick={() => (confirming() ? imported(preview()!.usable, preview()!.usable) : setConfirming(true))}
                    onBlur={() => setConfirming(false)}
                >
                    {confirming() ? "Click again to replace all" : "Replace all"}
                </Button>
                <Button onClick={p.onCancel}>Cancel</Button>
            </div>
        </div>
    );
}

/** Export and Import, and what they last did. */
export function KeyboardTransfer(p: { edit: KeyboardEdit }): JSX.Element {
    const [open, setOpen] = createSignal(false);
    const [note, setNote] = createSignal("");
    const exportNow = () => {
        const entries = p.edit.entries();
        if (entries.length === 0) return setNote("Nothing to export: you have no changes.");
        void writeText(exportText(entries)).then(
            () => setNote(`Copied ${entries.length} ${entries.length === 1 ? "entry" : "entries"}.`),
            () => setNote("Could not copy to the clipboard.")
        );
    };
    return (
        <>
            <Button onClick={exportNow}>Export</Button>
            <Button
                aria-expanded={open()}
                onClick={() => {
                    setNote("");
                    setOpen(!open());
                }}
            >
                Import…
            </Button>
            <span class="kb-transfer-note" role="status" aria-live="polite">
                {note()}
            </span>
            <Show when={open()}>
                <ImportPanel
                    edit={p.edit}
                    onDone={(n) => {
                        setOpen(false);
                        setNote(n);
                    }}
                    onCancel={() => setOpen(false)}
                />
            </Show>
        </>
    );
}
