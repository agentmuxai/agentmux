// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleImportConfirmModalPanel — Step 3 of the ABF import flow
 * (docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md §4 Step 3).
 *
 * Shows a summary of the current selection and calls `bundle.import.commit`
 * on "Import". A digest mismatch (the file changed since preview) surfaces
 * as a distinct, clearly-worded error directing the user back to step 1 —
 * not a generic failure. A partial failure (a skill skipped server-side
 * because of a last-second name conflict) is a real, expected outcome, not
 * an error state — it's surfaced via `warnings`/`skipped_skills` instead.
 */

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { BundleImportSelectionState } from "@/app/element/modal-layer";

interface BundleImportConfirmModalPanelProps {
    filePath: string;
    contentDigest: string;
    bundleDisplayName: string;
    selection: BundleImportSelectionState;
    onImported: (result: BundleImportCommitResponse) => void;
    onCancel: () => void;
}

function summaryLine(selection: BundleImportSelectionState): string {
    const parts: string[] = [];
    if (selection.includeInstructions) parts.push("instructions");
    if (selection.includeContextFileIds.length > 0) {
        parts.push(`${selection.includeContextFileIds.length} context file${selection.includeContextFileIds.length === 1 ? "" : "s"}`);
    }
    const includedSkills = selection.skills.filter((s) => s.checked && s.renameValue.trim().length >= 0);
    // A colliding-but-blank-rename row still shows as "selected" in the
    // checkbox UI but will be skipped server-side -- the summary counts
    // checked rows, matching what the user sees checked, not the final
    // server-side outcome (which the result screen reports separately).
    if (includedSkills.length > 0) {
        parts.push(`${includedSkills.length} skill${includedSkills.length === 1 ? "" : "s"}`);
    }
    if (selection.includeMcpServerPaths.length > 0) {
        parts.push(`${selection.includeMcpServerPaths.length} MCP server${selection.includeMcpServerPaths.length === 1 ? "" : "s"}`);
    }
    return parts.length > 0 ? `Importing: ${parts.join(", ")}.` : "Nothing selected to import.";
}

export const BundleImportConfirmModalPanel = (
    props: BundleImportConfirmModalPanelProps,
): JSX.Element => {
    const [submitting, setSubmitting] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const [digestMismatch, setDigestMismatch] = createSignal(false);
    const [result, setResult] = createSignal<BundleImportCommitResponse | null>(null);

    const commit = async () => {
        setSubmitting(true);
        setError(null);
        setDigestMismatch(false);
        try {
            const res = await RpcApi.BundleImportCommitCommand(TabRpcClient, {
                file_path: props.filePath,
                expected_content_digest: props.contentDigest,
                bundle_name: props.selection.bundleName,
                include_instructions: props.selection.includeInstructions,
                include_context_files: props.selection.includeContextFileIds,
                include_skills: props.selection.skills
                    .filter((s) => s.checked)
                    .map((s) => ({
                        source_dir: s.sourceDir,
                        ...(s.renameValue.trim() ? { import_as: s.renameValue.trim() } : {}),
                    })),
                include_mcp_servers: props.selection.includeMcpServerPaths,
            });
            setSubmitting(false);
            if (res.skipped_skills.length > 0 || res.warnings.length > 0) {
                // Real, expected partial-failure outcome -- show the
                // result inline rather than closing immediately.
                setResult(res);
            } else {
                props.onImported(res);
            }
        } catch (e) {
            setSubmitting(false);
            const message = (e as Error)?.message ?? String(e);
            if (message.includes("digest mismatch")) {
                setDigestMismatch(true);
            } else {
                setError(message);
            }
        }
    };

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">Confirm Import</h2>
                <p class="modal-panel-description">
                    Importing as <strong>{props.bundleDisplayName}</strong>.
                </p>
            </header>
            <div class="modal-panel-body bundle-import-confirm-body">
                <Show
                    when={!result()}
                    fallback={
                        <div class="bundle-import-result">
                            <p>Bundle imported.</p>
                            <Show when={result()!.skipped_skills.length > 0}>
                                <div class="bundle-import-hint bundle-import-hint-warn">
                                    Skipped skill(s): {result()!.skipped_skills.join(", ")}
                                </div>
                            </Show>
                            <Show when={result()!.warnings.length > 0}>
                                <div class="bundle-import-warnings-banner">
                                    <For each={result()!.warnings}>
                                        {(w) => <div class="bundle-import-warning-line">{w}</div>}
                                    </For>
                                </div>
                            </Show>
                        </div>
                    }
                >
                    <p class="bundle-import-summary-line">{summaryLine(props.selection)}</p>
                </Show>

                <Show when={digestMismatch()}>
                    <div class="bundle-import-error">
                        The file changed since it was previewed. Please cancel and re-select it.
                    </div>
                </Show>
                <Show when={error()}>
                    <div class="bundle-import-error">{error()}</div>
                </Show>
            </div>
            <footer class="modal-panel-footer">
                <Show
                    when={!result()}
                    fallback={
                        <Button onClick={() => props.onImported(result()!)} className="green solid">
                            Done
                        </Button>
                    }
                >
                    <Button onClick={() => props.onCancel()} disabled={submitting()} data-modal-dismiss>
                        Cancel
                    </Button>
                    <Button onClick={() => void commit()} className="green solid" disabled={submitting()}>
                        {submitting() ? "Importing…" : "Import"}
                    </Button>
                </Show>
            </footer>
        </>
    );
};

BundleImportConfirmModalPanel.displayName = "BundleImportConfirmModalPanel";
