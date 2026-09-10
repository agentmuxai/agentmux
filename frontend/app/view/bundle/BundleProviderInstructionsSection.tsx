// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleProviderInstructionsSection — authoring UI for ABF v0.2 §2.2's
 * `instructions_by_provider`: per-provider overrides of the bundle's default
 * Instructions, applied when the agent runs on that specific CLI/harness.
 *
 * Closes the last gap in ABF v0.2. Storage, export (`instructions/<provider>/
 * AGENTS.md`) and import all shipped in #2521/#2523/#2527; the spec committed
 * to shipping "the storage shape, the authoring UI, and export/import", and
 * only the authoring half was missing — an imported bundle's variants
 * round-tripped through the form untouched but could not be created or edited
 * in the app at all.
 *
 * WHY THIS IS A CONTROLLED/UNCONTROLLED HYBRID, which is the one subtle thing
 * here: the draft stores this field as a RAW JSON STRING, deliberately (see
 * `bundle-model.ts`'s seam comment — round-tripping the string verbatim is
 * what fixed reagent P1 on #2523, where editing any field silently wiped an
 * imported bundle's variants). Deriving the rows from that string on every
 * keystroke would make a row vanish the moment its provider key went blank,
 * because a blank key cannot be represented in a JSON object. So rows live in
 * local state and are serialized OUT to the draft on each edit, while an
 * effect re-seeds them if the incoming string changes for a reason other than
 * our own last write (switching bundles, a fresh load).
 */

import { createEffect, createMemo, createSignal, For, Index, Show, type JSX } from "solid-js";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { PROVIDERS } from "@/app/view/agent/providers/catalog";
import {
    duplicateProviderKeys,
    parseInstructionsByProvider,
    providerKeyProblem,
    serializeInstructionsByProvider,
    type ProviderInstruction,
} from "./bundle-model";

interface BundleProviderInstructionsSectionProps {
    /** The draft's raw `instructions_by_provider` JSON string. */
    value: string;
    onChange: (nextRaw: string) => void;
}

export const BundleProviderInstructionsSection = (
    props: BundleProviderInstructionsSectionProps,
): JSX.Element => {
    const parsed = createMemo(() => parseInstructionsByProvider(props.value));

    const [rows, setRows] = createSignal<ProviderInstruction[]>(parsed().entries);
    // What we last wrote out, so the re-seed effect can tell an external change
    // (bundle switch) apart from the echo of our own edit.
    let lastEmitted = props.value;

    createEffect(() => {
        const incoming = props.value;
        if (incoming === lastEmitted) return;
        lastEmitted = incoming;
        setRows(parseInstructionsByProvider(incoming).entries);
    });

    const emit = (next: ProviderInstruction[]) => {
        setRows(next);
        const raw = serializeInstructionsByProvider(next);
        lastEmitted = raw;
        props.onChange(raw);
    };

    const updateRow = (index: number, patch: Partial<ProviderInstruction>) => {
        emit(rows().map((r, i) => (i === index ? { ...r, ...patch } : r)));
    };

    const removeRow = (index: number) => {
        emit(rows().filter((_, i) => i !== index));
    };

    const addRow = () => {
        // Blank key by design: the user names it. It is flagged immediately and
        // simply does not serialize until named, so an unnamed row can never
        // reach storage or export.
        emit([...rows(), { provider: "", content: "" }]);
    };

    const dupes = createMemo(() => new Set(duplicateProviderKeys(rows())));
    const knownProviders = createMemo(() => Object.keys(PROVIDERS).sort());

    return (
        <div class="bundle-view-provider-instructions">
            <Show
                when={!parsed().malformed}
                fallback={
                    // Refusing to edit is the correct behavior, not a cop-out:
                    // rendering an empty editor here would serialize `{}` over
                    // the stored value on the next save and destroy variants we
                    // merely failed to parse. The raw string is preserved
                    // untouched, so an export/re-import or a manual fix
                    // recovers it.
                    <div class="bundle-view-provider-instructions-malformed">
                        This bundle's per-provider instructions are not readable as a JSON
                        object of provider → text, so they cannot be edited here. The stored
                        value is preserved exactly as-is and will survive saving this form —
                        nothing is lost. Export the bundle to inspect or repair it.
                    </div>
                }
            >
                {/* <Index>, NOT <For>: these rows are index-addressed and every
                    edit replaces the row object, which <For> keys by identity —
                    so it would dispose and recreate the row's DOM on every
                    keystroke, dropping focus after each character and making
                    the field unusable for normal typing (codex P1, #3063).
                    <Index> keys by position and hands the item in as an
                    accessor, so the input element survives edits. */}
                <Index each={rows()}>
                    {(row, index) => {
                        const problem = () => providerKeyProblem(row().provider);
                        const duped = () => dupes().has(row().provider.trim());  // keyed on the raw trimmed key, which is what duplicateProviderKeys reports
                        return (
                            <div class="bundle-view-provider-instruction-row">
                                <div class="bundle-view-provider-instruction-head">
                                    <input
                                        class="bundle-view-input"
                                        list="abf-known-providers"
                                        placeholder="Provider (e.g. claude)"
                                        value={row().provider}
                                        onInput={(e) =>
                                            updateRow(index, { provider: e.currentTarget.value })
                                        }
                                        onContextMenu={showTextInputContextMenu}
                                    />
                                    <button
                                        type="button"
                                        class="bundle-view-provider-instruction-remove"
                                        onClick={() => removeRow(index)}
                                        title="Remove this provider override"
                                    >
                                        Remove
                                    </button>
                                </div>
                                <Show when={problem()}>
                                    {/* The consequence differs by case and the
                                        warning has to match, or it is just a
                                        different wrong message (reagent P1,
                                        #3063 said "It will not be saved" for
                                        every problem, which was false for most
                                        of them):
                                        - blank key: genuinely not saved,
                                          serializeInstructionsByProvider drops
                                          it;
                                        - any other rejected key: saved fine,
                                          but bundle_export.rs skips it with a
                                          warning, so it vanishes from the .abf. */}
                                    <div class="bundle-view-provider-instruction-warn">
                                        {problem()}{" "}
                                        {row().provider.trim().length === 0
                                            ? "This row will not be saved until you name it."
                                            : "It will still be saved, but skipped when this bundle is exported."}
                                    </div>
                                </Show>
                                <Show when={!problem() && duped()}>
                                    <div class="bundle-view-provider-instruction-warn">
                                        Two keys resolve to the same export path — only one of
                                        them survives export.
                                    </div>
                                </Show>
                                <textarea
                                    class="bundle-view-textarea"
                                    rows={5}
                                    value={row().content}
                                    onInput={(e) =>
                                        updateRow(index, { content: e.currentTarget.value })
                                    }
                                    onContextMenu={showTextInputContextMenu}
                                    placeholder="Instructions used only when this bundle runs on this provider."
                                />
                            </div>
                        );
                    }}
                </Index>

                <datalist id="abf-known-providers">
                    <For each={knownProviders()}>{(p) => <option value={p} />}</For>
                </datalist>

                <button type="button" class="bundle-view-provider-instruction-add" onClick={addRow}>
                    + Add provider override
                </button>
            </Show>
        </div>
    );
};

BundleProviderInstructionsSection.displayName = "BundleProviderInstructionsSection";
