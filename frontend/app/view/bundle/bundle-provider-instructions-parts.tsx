// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Per-provider instruction overrides, the views: one provider's row (its key, Remove, what is wrong with the key, its
// text), the list of known providers, and the add button, over the state of bundle-provider-instructions-state.ts.

import { For, Show, type Accessor, type JSX } from "solid-js";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { Button, TextArea, TextInput } from "@/app/element/ui";
import { focusWhenRendered } from "@/util/focusutil";
import { providerKeyProblem, type ProviderInstruction } from "./bundle-model";
import type { ProviderInstructionsState } from "./bundle-provider-instructions-state";

/** One provider's override: its key and Remove, the warning its key earns, and its instructions. */
export function InstructionRow(props: { s: ProviderInstructionsState; row: Accessor<ProviderInstruction>; index: number }): JSX.Element {
    const { s, row, index } = props;
    const problem = () => providerKeyProblem(row().provider);
    const duped = () => s.dupes().has(row().provider.trim());  // keyed on the raw trimmed key, which is what duplicateProviderKeys reports
    return (
        <div class="bundle-view-provider-instruction-row">
            <div class="bundle-view-provider-instruction-head">
                <TextInput variant="plain"
                    class="bundle-view-input"
                    list="abf-known-providers"
                    placeholder="Provider (e.g. claude)"
                    value={row().provider}
                    onInput={(e) =>
                        s.updateRow(index, { provider: e.currentTarget.value })
                    }
                    onContextMenu={showTextInputContextMenu}
                />
                <Button
                    class="bundle-view-provider-instruction-remove"
                    onClick={() => s.removeRow(index)}
                    title="Remove this provider override"
                >
                    Remove
                </Button>
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
            <TextArea variant="plain"
                class="bundle-view-textarea"
                rows={5}
                value={row().content}
                onInput={(e) =>
                    s.updateRow(index, { content: e.currentTarget.value })
                }
                onContextMenu={showTextInputContextMenu}
                placeholder="Instructions used only when this bundle runs on this provider."
            />
        </div>
    );
}

/** The providers the key box suggests. */
export function KnownProviders(props: { s: ProviderInstructionsState }): JSX.Element {
    return (
        <datalist id="abf-known-providers">
            <For each={props.s.knownProviders()}>{(p) => <option value={p} />}</For>
        </datalist>
    );
}

/** Adds a row and puts the focus in its key box. */
export function AddOverrideButton(props: { s: ProviderInstructionsState }): JSX.Element {
    return (
        <Button
            icon="plus"
            class="bundle-view-provider-instruction-add"
            onClick={(e) => {
                const section = e.currentTarget.parentElement;
                props.s.addRow();
                focusWhenRendered(() =>
                    section?.querySelector<HTMLInputElement>(".bundle-view-provider-instruction-row:last-of-type input")
                );
            }}
        >
            Add provider override
        </Button>
    );
}
