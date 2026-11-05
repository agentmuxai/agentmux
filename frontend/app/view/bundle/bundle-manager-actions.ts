// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the bundle manager does on a click: select, create, save, validate, delete, import and go back. */

import { createConfirm } from "@/app/element/create-confirm";
import { useModalLayer } from "@/app/element/modal-layer";
import type { Bundle } from "@/app/store/rpc-api";
import { type BundleDraft, BundleViewModel } from "./bundle-model";

type ModalLayerApi = ReturnType<typeof useModalLayer>;

// Starts the 3-step ABF import chain (SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md
// §4) -- each step's request builds the next via modalLayer.replace(),
// mirroring the install→launch chain precedent. The commit RPC
// publishes `memories:changed`, which this model already subscribes
// to, so the newly-imported bundle appears in the list with no extra
// glue here.
function openBundleImport(modalLayer: ModalLayerApi) {
    modalLayer.open({
        kind: "bundle-import-select",
        onPreviewed: (filePath, preview) => {
            modalLayer.replace({
                kind: "bundle-import-preview",
                filePath,
                preview,
                onNext: (selection) => {
                    modalLayer.replace({
                        kind: "bundle-import-confirm",
                        filePath,
                        contentDigest: preview.content_digest,
                        bundleDisplayName: selection.bundleName,
                        selection,
                        onImported: () => modalLayer.close(),
                        onCancel: modalLayer.close,
                    });
                },
                onCancel: modalLayer.close,
            });
        },
        onCancel: modalLayer.close,
    });
}

export function createBundleActions(model: BundleViewModel) {
    const modalLayer = useModalLayer();

    const handleImportBundle = () => openBundleImport(modalLayer);

    // Clicking a list item SELECTS the memory so the read-only detail
    // view appears (including for the blank singleton, which can't be
    // edited but should still be inspectable). The edit form opens via
    // the explicit "Edit" button on the read-only view. (Selecting used to call startEdit,
    // which refused on the blank singleton with an error and left the detail pane showing
    // the previously-selected memory, mismatching the banner; #749.)
    const handleSelect = (memory: Bundle) => {
        model.setError(null);
        model.cancelDraft();
        model.setSelectedId(memory.id);
    };

    const handleNew = () => model.startNew();

    const handleSave = () => {
        void model.saveDraft();
    };

    const handleCancel = () => model.cancelDraft();

    const handleValidate = () => {
        void model.validateDraft();
    };

    const confirm = createConfirm();
    const handleDelete = async (id: string) => {
        if (await confirm.ask({ title: "Delete this bundle?", description: "Running instances continue with their snapshot, but new launches won't see it.", confirmLabel: "Delete" })) void model.deleteMemory(id);
    };

    const updateDraft = <K extends keyof BundleDraft>(
        key: K,
        value: BundleDraft[K],
    ) => {
        const current = model.draftAtom();
        if (!current) return;
        model.setDraft({ ...current, [key]: value });
        // A validation report reflects the draft AS OF the click; any edit
        // after that invalidates it — clear rather than leave a stale
        // "looks good" (or stale error) hanging off text the user changed.
        model.setValidation(null);
    };

    // Single-pane: the detail view (read-only or edit form) shows instead of
    // the list, never alongside it — see
    // docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md.
    // "In detail" whenever something is selected OR a draft (new or
    // editing-existing) is open; otherwise the list shows. The old
    // no-selection empty-state message ("Select a bundle...") is gone — with
    // nothing selected you're just looking at the list, there's no third
    // empty detail state to explain.
    const inDetail = () => model.selectedIdAtom() !== null || model.draftAtom() !== null;

    const handleBack = () => {
        model.setError(null);
        model.setSelectedId(null);
        model.setDraft(null);
    };

    return {
        confirm,
        handleImportBundle,
        handleSelect,
        handleNew,
        handleSave,
        handleCancel,
        handleValidate,
        handleDelete,
        updateDraft,
        inDetail,
        handleBack,
    };
}

export type BundleActions = ReturnType<typeof createBundleActions>;
