// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleImportSelectModalPanel — Step 1 of the ABF import flow
 * (docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md §4 Step 1).
 *
 * Has almost no UI of its own: it triggers the native `.abf` file dialog
 * on mount, calls `bundle.import.preview` with the picked path, and
 * either advances (`onPreviewed`) or shows the parse/validation error
 * inline with a "Choose a different file" retry — per the spec, an error
 * here surfaces on THIS step rather than advancing.
 */

import { createSignal, onMount, Show, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { getApi } from "@/app/store/app-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

interface BundleImportSelectModalPanelProps {
    onPreviewed: (filePath: string, preview: BundleImportPreviewResponse) => void;
    onCancel: () => void;
}

export const BundleImportSelectModalPanel = (
    props: BundleImportSelectModalPanelProps,
): JSX.Element => {
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    // Once the user picks a file (or explicitly cancels the OS dialog
    // twice with nothing chosen), we don't want to keep re-firing the
    // native dialog on every render -- track whether we've prompted yet.
    const [awaitingPick, setAwaitingPick] = createSignal(true);

    const pickAndPreview = async () => {
        setError(null);
        setAwaitingPick(false);
        const path = await getApi()?.showOpenBundleDialog?.();
        if (!path) {
            // User cancelled the OS dialog with nothing chosen -- close
            // the whole flow rather than leaving an empty modal open.
            props.onCancel();
            return;
        }
        setBusy(true);
        try {
            const preview = await RpcApi.BundleImportPreviewCommand(TabRpcClient, { file_path: path });
            setBusy(false);
            props.onPreviewed(path, preview);
        } catch (e) {
            setBusy(false);
            setError((e as Error)?.message ?? String(e));
        }
    };

    onMount(() => {
        void pickAndPreview();
    });

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">Import Bundle</h2>
                <p class="modal-panel-description">
                    Pick an Armory Bundle (<code>.abf</code>) file to see what's inside before importing.
                </p>
            </header>
            <div class="modal-panel-body bundle-import-select-body">
                <Show when={busy()}>
                    <div class="bundle-import-select-status">Reading bundle…</div>
                </Show>
                <Show when={error()}>
                    <div class="bundle-import-select-error">{error()}</div>
                </Show>
                <Show when={!busy() && !awaitingPick()}>
                    <Button onClick={() => void pickAndPreview()} className="green">
                        Choose a different file
                    </Button>
                </Show>
            </div>
            <footer class="modal-panel-footer">
                <Button onClick={() => props.onCancel()} disabled={busy()} data-modal-dismiss>
                    Cancel
                </Button>
            </footer>
        </>
    );
};

BundleImportSelectModalPanel.displayName = "BundleImportSelectModalPanel";
