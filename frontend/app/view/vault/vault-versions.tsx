// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An entry's earlier values, kept 7 days after they were replaced, and Restore
// (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.4).
// No value is shown: when, by whom, and the last characters, as the list does.

import { createResource, createSignal, For, Show, type JSX } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button } from "@/app/element/ui";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import { formatWhen } from "./vault-format";
import { loadVersions, restoreVersion } from "./vault-store";

export function VaultVersionsDialog(props: { entry: VaultEntryInfo; onClose: () => void }): JSX.Element {
    const [versions, { refetch }] = createResource(() => props.entry.id, loadVersions);
    const [busy, setBusy] = createSignal<number | null>(null);
    const [error, setError] = createSignal<string | null>(null);

    const restore = async (seq: number) => {
        setBusy(seq);
        try {
            await restoreVersion(props.entry.id, seq);
            setError(null);
            await refetch();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
        } finally {
            setBusy(null);
        }
    };

    return (
        <Modal open={true} scope="pane" onClose={props.onClose} size="md" panelClass="vault-form">
            <ModalHeader
                title={`Earlier values of ${props.entry.label}`}
                description="Each is kept for 7 days after it was replaced. Restore makes it the value agents use; the current one is kept here in its place."
            />
            <ModalBody>
                <Show when={error()}>
                    <div class="warden-manager-error" role="alert">
                        ⚠ {error()}
                    </div>
                </Show>
                <Show when={(versions() ?? []).length > 0} fallback={<div class="warden-section-stub">{versions.loading ? "Loading…" : "No earlier value is kept."}</div>}>
                    <div class="vault-versions">
                        <For each={versions()}>
                            {(v) => (
                                <div class="vault-version" data-seq={v.seq}>
                                    <span class="warden-manager-mono">{v.tail ? `•••• ${v.tail}` : "••••"}</span>
                                    <span class="warden-manager-dim">
                                        {v.how} by {v.replaced_by}, {formatWhen(v.replaced_at)}
                                    </span>
                                    <Button density="compact" icon="rotate-left" busy={busy() === v.seq} onClick={() => void restore(v.seq)}>
                                        Restore
                                    </Button>
                                </div>
                            )}
                        </For>
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.onClose}>
                    Close
                </Button>
            </ModalFooter>
        </Modal>
    );
}
