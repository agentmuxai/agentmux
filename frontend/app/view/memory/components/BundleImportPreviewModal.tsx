// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleImportPreviewModalPanel — Step 2 of the ABF import flow
 * (docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md §4 Step 2 / §4.1).
 *
 * Renders the `bundle.import.preview` response as a selection checklist.
 * All selection state is built here and handed to Step 3 via `onNext` —
 * this panel makes no RPC calls of its own.
 */

import { createSignal, For, onMount, Show, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { BundleImportSelectionState, BundleImportSkillSelectionState } from "@/app/element/modal-layer";

interface BundleImportPreviewModalPanelProps {
    preview: BundleImportPreviewResponse;
    onNext: (selection: BundleImportSelectionState) => void;
    onCancel: () => void;
}

function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export const BundleImportPreviewModalPanel = (
    props: BundleImportPreviewModalPanelProps,
): JSX.Element => {
    const { preview } = props;

    const [bundleName, setBundleName] = createSignal(preview.name);
    const [includeInstructions, setIncludeInstructions] = createSignal(true);
    const [instructionsExpanded, setInstructionsExpanded] = createSignal(false);
    const [contextChecked, setContextChecked] = createSignal<Record<number, boolean>>(
        Object.fromEntries(preview.context_files.map((cf) => [cf.id, true])),
    );
    const [skills, setSkills] = createSignal<BundleImportSkillSelectionState[]>(
        preview.skills.map((s) => ({ sourceDir: s.source_dir, checked: true, renameValue: "" })),
    );
    const [mcpChecked, setMcpChecked] = createSignal<Record<string, boolean>>(
        Object.fromEntries(preview.mcp_servers.map((m) => [m.source_path, true])),
    );
    const [warningsDismissed, setWarningsDismissed] = createSignal(false);

    // §4.1 point 2: the modal fetches the full existing global skill-name
    // list ONCE, up front, via skill.catalog.list -- not just the subset
    // preview.skills already flagged "name_conflict" against itself (a
    // rename to some OTHER, unrelated existing global skill name would
    // otherwise show no client-side warning and silently be skipped at
    // commit, since skill_upsert_unique_global still rejects it there).
    const [globalSlugs, setGlobalSlugs] = createSignal<Set<string>>(new Set());
    onMount(() => {
        void RpcApi.SkillCatalogListCommand(TabRpcClient, {})
            .then((items) => setGlobalSlugs(new Set(items.map((i) => i.name))))
            .catch(() => {}); // advisory only -- a failed fetch just means no client-side hint; commit is still authoritative.
    });

    // Live union of the global catalog + every OTHER selected skill's
    // current slug/rename (§4.1 point 2) -- advisory client-side check
    // only; skill_upsert_unique_global is the real authority at commit.
    const takenSlugsFor = (sourceDir: string): Set<string> => {
        const taken = new Set<string>(globalSlugs());
        for (const row of skills()) {
            if (row.sourceDir === sourceDir || !row.checked) continue;
            const other = preview.skills.find((s) => s.source_dir === row.sourceDir);
            const effective = row.renameValue.trim() || other?.slug;
            if (effective) taken.add(effective);
        }
        return taken;
    };

    const updateSkill = (sourceDir: string, patch: Partial<BundleImportSkillSelectionState>) => {
        setSkills((prev) => prev.map((s) => (s.sourceDir === sourceDir ? { ...s, ...patch } : s)));
    };

    const toggleContext = (id: number) => {
        setContextChecked((prev) => ({ ...prev, [id]: !prev[id] }));
    };

    const toggleMcp = (path: string) => {
        setMcpChecked((prev) => ({ ...prev, [path]: !prev[path] }));
    };

    const suggestedAltName = () => {
        const m = bundleName().match(/^(.*) \((\d+)\)$/);
        if (m) return `${m[1]} (${Number(m[2]) + 1})`;
        return `${bundleName()} (2)`;
    };

    const next = () => {
        props.onNext({
            bundleName: bundleName().trim() || preview.name,
            includeInstructions: includeInstructions(),
            includeContextFileIds: preview.context_files
                .filter((cf) => contextChecked()[cf.id])
                .map((cf) => cf.id),
            skills: skills(),
            includeMcpServerPaths: preview.mcp_servers
                .filter((m) => mcpChecked()[m.source_path])
                .map((m) => m.source_path),
        });
    };

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">Preview &amp; Select</h2>
                <p class="modal-panel-description">
                    Choose what to bring in from <strong>{preview.name}</strong>.
                </p>
            </header>
            <div class="modal-panel-body bundle-import-preview-body">
                <label class="bundle-import-field">
                    <span class="bundle-import-field-label">Bundle name</span>
                    <input
                        type="text"
                        class="bundle-import-input"
                        value={bundleName()}
                        onInput={(e) => setBundleName(e.currentTarget.value)}
                    />
                    <Show when={preview.name_collision}>
                        <div class="bundle-import-hint">
                            A bundle named this already exists.{" "}
                            <button
                                type="button"
                                class="bundle-import-link-btn"
                                onClick={() => setBundleName(suggestedAltName())}
                            >
                                Use "{suggestedAltName()}" instead
                            </button>
                        </div>
                    </Show>
                </label>

                <section class="bundle-import-section">
                    <label class="bundle-import-checkbox-row">
                        <input
                            type="checkbox"
                            checked={includeInstructions()}
                            onChange={(e) => setIncludeInstructions(e.currentTarget.checked)}
                        />
                        <span>Include instructions</span>
                    </label>
                    <Show when={includeInstructions() && preview.instructions_preview}>
                        <div class="bundle-import-instructions-preview">
                            <button
                                type="button"
                                class="bundle-import-link-btn"
                                onClick={() => setInstructionsExpanded((v) => !v)}
                            >
                                {instructionsExpanded() ? "Hide" : "Show"} preview
                            </button>
                            <Show when={instructionsExpanded()}>
                                <pre class="bundle-import-instructions-text">{preview.instructions_preview}</pre>
                                <Show when={preview.instructions_truncated}>
                                    <div class="bundle-import-hint">
                                        Preview truncated — {preview.instructions_total_chars} characters total,
                                        the full instructions are still imported.
                                    </div>
                                </Show>
                            </Show>
                        </div>
                    </Show>
                </section>

                <Show when={preview.context_files.length > 0}>
                    <section class="bundle-import-section">
                        <h3 class="bundle-import-section-title">Context files</h3>
                        <For each={preview.context_files}>
                            {(cf) => (
                                <label class="bundle-import-checkbox-row">
                                    <input
                                        type="checkbox"
                                        checked={!!contextChecked()[cf.id]}
                                        onChange={() => toggleContext(cf.id)}
                                    />
                                    <span class="bundle-import-item-name">{cf.display_path}</span>
                                    <span class="bundle-import-item-meta">{formatBytes(cf.size_bytes)}</span>
                                </label>
                            )}
                        </For>
                    </section>
                </Show>

                <Show when={preview.skills.length > 0}>
                    <section class="bundle-import-section">
                        <h3 class="bundle-import-section-title">Skills</h3>
                        <For each={preview.skills}>
                            {(skill) => {
                                const row = () => skills().find((s) => s.sourceDir === skill.source_dir)!;
                                const colliding = skill.collision !== "none";
                                const renameConflict = () =>
                                    colliding &&
                                    row().renameValue.trim().length > 0 &&
                                    takenSlugsFor(skill.source_dir).has(row().renameValue.trim());
                                return (
                                    <div class="bundle-import-skill-row">
                                        <label class="bundle-import-checkbox-row">
                                            <input
                                                type="checkbox"
                                                checked={row().checked}
                                                onChange={(e) =>
                                                    updateSkill(skill.source_dir, { checked: e.currentTarget.checked })
                                                }
                                            />
                                            <Show
                                                when={!colliding}
                                                fallback={
                                                    <span class="bundle-import-skill-collision">
                                                        <span class="bundle-import-collision-badge">
                                                            {skill.collision === "name_conflict"
                                                                ? "already exists in your library"
                                                                : "another skill in this import uses this name"}
                                                        </span>
                                                        <span class="bundle-import-item-name-readonly">{skill.slug}</span>
                                                        <input
                                                            type="text"
                                                            class="bundle-import-rename-input"
                                                            placeholder="rename to import (leave blank to skip)"
                                                            value={row().renameValue}
                                                            disabled={!row().checked}
                                                            onInput={(e) =>
                                                                updateSkill(skill.source_dir, { renameValue: e.currentTarget.value })
                                                            }
                                                        />
                                                    </span>
                                                }
                                            >
                                                <span class="bundle-import-item-name">{skill.slug}</span>
                                            </Show>
                                        </label>
                                        <Show when={skill.description}>
                                            <p class="bundle-import-item-description">{skill.description}</p>
                                        </Show>
                                        <Show when={renameConflict()}>
                                            <div class="bundle-import-hint bundle-import-hint-warn">
                                                This name is also taken — pick another.
                                            </div>
                                        </Show>
                                    </div>
                                );
                            }}
                        </For>
                    </section>
                </Show>

                <Show when={preview.mcp_servers.length > 0}>
                    <section class="bundle-import-section">
                        <h3 class="bundle-import-section-title">MCP servers</h3>
                        <For each={preview.mcp_servers}>
                            {(m) => (
                                <label class="bundle-import-checkbox-row">
                                    <input
                                        type="checkbox"
                                        checked={!!mcpChecked()[m.source_path]}
                                        onChange={() => toggleMcp(m.source_path)}
                                    />
                                    <span class="bundle-import-item-name">
                                        {m.display.name ?? m.source_path.split("/").pop()}
                                    </span>
                                    <Show when={m.display.command}>
                                        <span class="bundle-import-item-meta">{m.display.command}</span>
                                    </Show>
                                </label>
                            )}
                        </For>
                    </section>
                </Show>

                <Show when={preview.requirements.length > 0}>
                    <section class="bundle-import-section">
                        <h3 class="bundle-import-section-title">Account requirements</h3>
                        <p class="bundle-import-requirements-summary">
                            Depends on {preview.requirements.length} account(s):{" "}
                            {preview.requirements
                                .map((r) => `${r.provider} (${r.resolved ? "resolved" : "not connected"})`)
                                .join(", ")}
                        </p>
                    </section>
                </Show>

                <Show when={preview.warnings.length > 0 && !warningsDismissed()}>
                    <div class="bundle-import-warnings-banner">
                        <button
                            type="button"
                            class="bundle-import-warnings-dismiss"
                            onClick={() => setWarningsDismissed(true)}
                            aria-label="Dismiss warnings"
                        >
                            ×
                        </button>
                        <For each={preview.warnings}>{(w) => <div class="bundle-import-warning-line">{w}</div>}</For>
                        <Show when={preview.warnings_truncated}>
                            <div class="bundle-import-warning-line">…more warnings not shown</div>
                        </Show>
                    </div>
                </Show>
            </div>
            <footer class="modal-panel-footer">
                <Button onClick={() => props.onCancel()} data-modal-dismiss>
                    Cancel
                </Button>
                <Button onClick={next} className="green solid">
                    Next
                </Button>
            </footer>
        </>
    );
};

BundleImportPreviewModalPanel.displayName = "BundleImportPreviewModalPanel";
