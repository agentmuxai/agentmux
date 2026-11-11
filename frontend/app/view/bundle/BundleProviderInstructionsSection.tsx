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

import { Index, Show, type JSX } from "solid-js";
import { AddOverrideButton, InstructionRow, KnownProviders } from "./bundle-provider-instructions-parts";
import { createProviderInstructions, type BundleProviderInstructionsSectionProps } from "./bundle-provider-instructions-state";

export const BundleProviderInstructionsSection = (
    props: BundleProviderInstructionsSectionProps,
): JSX.Element => {
    const s = createProviderInstructions(props);

    return (
        <div class="bundle-view-provider-instructions">
            <Show
                when={!s.parsed().malformed}
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
                <Index each={s.rows()}>
                    {(row, index) => <InstructionRow s={s} row={row} index={index} />}
                </Index>

                <KnownProviders s={s} />

                <AddOverrideButton s={s} />
            </Show>
        </div>
    );
};

BundleProviderInstructionsSection.displayName = "BundleProviderInstructionsSection";
