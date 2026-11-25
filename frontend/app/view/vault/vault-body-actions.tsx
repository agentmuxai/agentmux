// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Import, Export and the note on what may ask the person: buttons in the Vault's own row, beside its Vaults, and not in the
// pane's tab. A narrow pane (vault-model.ts's `narrow`) shows the two transfers as icons, with the same titles.

import { Show, type JSX } from "solid-js";
import { Button, IconButton } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultViewModel } from "./vault-model";

const IMPORT = "Import into the Vault";
const EXPORT = "Export the Vault";
const NOTE = "What the Vault may ask you";

export function VaultBodyActions(props: { model: VaultViewModel }): JSX.Element {
    const model = () => props.model;
    return (
        <div class="vault-body-actions">
            <Show
                when={!model().narrow()}
                fallback={
                    <>
                        <IconButton density="compact" icon="file-import" label={IMPORT} onClick={() => model().openTransfer("import")} />
                        <IconButton density="compact" icon="file-export" label={EXPORT} onClick={() => model().openTransfer("export")} />
                    </>
                }
            >
                <Button density="compact" icon="file-import" title={IMPORT} aria-label={IMPORT} onClick={() => model().openTransfer("import")}>
                    {t("vault.import")}
                </Button>
                <Button density="compact" icon="file-export" title={EXPORT} aria-label={EXPORT} onClick={() => model().openTransfer("export")}>
                    {t("vault.export")}
                </Button>
            </Show>
            <IconButton density="compact" icon="circle-info" label={NOTE} onClick={() => model().setMeta({ "vault:prompts-read": null })} />
        </div>
    );
}
