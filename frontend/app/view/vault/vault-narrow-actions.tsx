// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Import, Export and the note on what may ask the person, in the Vault's own row when the pane is
// too narrow for them in its header (vault-model.ts's `narrow`): the same three buttons, the same
// actions, so nothing the header offers is lost.

import { For, type JSX } from "solid-js";

import { IconButton } from "@/app/element/ui";
import { vaultHeaderActions } from "./vault-header-actions";
import type { VaultViewModel } from "./vault-model";

export function VaultNarrowActions(props: { model: VaultViewModel }): JSX.Element {
    return (
        <div class="vault-narrow-actions">
            <For each={vaultHeaderActions(props.model)}>
                {(action) => <IconButton density="compact" icon={String(action.icon)} label={action.title ?? ""} onClick={(e) => action.click?.(e)} />}
            </For>
        </div>
    );
}
