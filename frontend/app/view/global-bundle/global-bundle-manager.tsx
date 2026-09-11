// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GlobalBundleManager — the Global section of the Armory "Memory" tab (moves to
// the Bundles tab in Phase 4 of SPEC_ARMORY_NAMING_CONSOLIDATION_2026_09_09.md).
// Presents the workspace-wide global bundles (is_global rows) as an ordered
// list of editable sections that compose into every agent's startup
// instructions file (CLAUDE.md, GEMINI.md, or similar, depending on
// provider) at launch.
//
// Context-free: owns its own GlobalBundleViewModel and drives off the
// bundle_* RPCs. Spec: docs/specs/archive/SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md.

import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { Markdown } from "@/app/element/markdown";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { GlobalBundleViewModel, NEW_SECTION_ID } from "./global-bundle-model";
import "./global-bundle.scss";

/** Inline editor card shared by the "new section" and "edit section" flows. */
function SectionEditor(props: { model: GlobalBundleViewModel; isNew: boolean }): JSX.Element {
    const { model } = props;
    return (
        <div class="global-bundle-editor">
            <label class="global-bundle-field">
                <span class="global-bundle-field-label">Name</span>
                <input
                    class="global-bundle-input"
                    type="text"
                    value={model.draftNameAtom()}
                    onInput={(e) => model.setDraftName(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="e.g. Coding Standards"
                />
            </label>
            <label class="global-bundle-field">
                <span class="global-bundle-field-label">Content</span>
                <textarea
                    class="global-bundle-textarea"
                    rows={8}
                    value={model.draftInstructionsAtom()}
                    onInput={(e) => model.setDraftInstructions(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="Markdown injected into every agent's startup instructions file under a # [Workspace] heading."
                    spellcheck={false}
                />
            </label>
            <div class="global-bundle-editor-actions">
                <button
                    class="global-bundle-btn"
                    disabled={model.savingAtom()}
                    onClick={() => model.cancelEdit()}
                >
                    Cancel
                </button>
                <button
                    class="global-bundle-btn global-bundle-btn-primary"
                    disabled={model.savingAtom() || !model.draftNameAtom().trim()}
                    onClick={() => void model.saveEdit()}
                >
                    {model.savingAtom() ? "Saving…" : props.isNew ? "Add section" : "Save"}
                </button>
            </div>
        </div>
    );
}

/** Inline editor card for the system tier — a separate component (not a
 *  parameterized SectionEditor) so its state is never accidentally wired to
 *  the ordinary draft signals. See
 *  docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §3.5. */
function SystemSectionEditor(props: { model: GlobalBundleViewModel; isNew: boolean }): JSX.Element {
    const { model } = props;
    return (
        <div class="global-bundle-editor global-bundle-editor-system">
            <label class="global-bundle-field">
                <span class="global-bundle-field-label">Name</span>
                <input
                    class="global-bundle-input"
                    type="text"
                    value={model.draftSystemNameAtom()}
                    onInput={(e) => model.setDraftSystemName(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="e.g. AgentMux Policy"
                />
            </label>
            <label class="global-bundle-field">
                <span class="global-bundle-field-label">Content</span>
                <textarea
                    class="global-bundle-textarea"
                    rows={8}
                    value={model.draftSystemInstructionsAtom()}
                    onInput={(e) => model.setDraftSystemInstructions(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="Markdown injected FIRST into every agent's startup instructions file, wrapped in explicit override wording."
                    spellcheck={false}
                />
            </label>
            <div class="global-bundle-editor-actions">
                <button
                    class="global-bundle-btn"
                    disabled={model.savingAtom()}
                    onClick={() => model.cancelEditSystem()}
                >
                    Cancel
                </button>
                <button
                    class="global-bundle-btn global-bundle-btn-primary"
                    disabled={model.savingAtom() || !model.draftSystemNameAtom().trim()}
                    onClick={() => void model.saveSystemEdit()}
                >
                    {model.savingAtom() ? "Saving…" : props.isNew ? "Add system entry" : "Save"}
                </button>
            </div>
        </div>
    );
}

export const GlobalBundleManager = (): JSX.Element => {
    const model = new GlobalBundleViewModel();
    onCleanup(() => model.dispose());

    const [promoteValue, setPromoteValue] = createSignal("");

    const handlePromote = (id: string) => {
        if (!id) return;
        void model.promote(id);
        setPromoteValue("");
    };

    return (
        <div class="global-bundle">
            <p class="global-bundle-intro">
                Inherited by every agent at launch — composed into its startup file (e.g.{" "}
                <code>CLAUDE.md</code>) in order.
            </p>

            <div class="global-bundle-restart-note">Takes effect on next agent restart.</div>

            <Show when={model.errorAtom()}>
                <div class="global-bundle-error">{model.errorAtom()}</div>
            </Show>

            {/* Read-only reference display of the CLAUDE.md in the isolated
                provider config dir a default spawned agent actually launches
                with (CLAUDE_CONFIG_DIR). NOT part of AgentMux's own Global
                Memory composition (that's <agent working_directory>/CLAUDE.md,
                a per-agent path already previewed accurately above via the
                "Combined preview"). See
                docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §7.

                The sibling "Claude Code — host CLI config" block (~/.claude)
                was removed 2026-09-01
                (SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md): once
                REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md
                proved by experiment that a spawned agent never reads the host
                file, surfacing it here was noise that invited the misreading
                that editing it would change agent behaviour. */}
            {/* Single <Show>: this used to be nested, the outer gating the
                whole section on `globalConfig || hostConfig` and the inner
                picking out this one block. With the host block gone both
                conditions collapsed to the same atom, leaving the inner one
                unreachable-when-false (ReAgent P2, PR #2900). The callback
                form supplies the non-null accessor the body needs. */}
            <Show when={model.claudeGlobalConfigAtom()}>
                {(cfg) => (
                    <div class="global-bundle-external-files">
                        <p class="global-bundle-external-files-heading">
                            Claude Code provider config — reference only, not part of Global Memory.
                        </p>

                        <div class="global-bundle-machine-config">
                            <div class="global-bundle-machine-config-header">
                                <span
                                    class="global-bundle-machine-config-badge"
                                    title="Hand-maintained on disk. Identity-bound agents use a separate dir, not shown here."
                                >
                                    Claude Code — shared provider config
                                </span>
                                <code class="global-bundle-machine-config-path">{cfg().path}</code>
                            </div>
                            <p class="global-bundle-machine-config-caption">Used by default spawned agents.</p>
                            <Show
                                when={cfg().exists}
                                fallback={<p class="global-bundle-machine-config-empty">No file at this path yet.</p>}
                            >
                                {/* The resize handle lives on this wrapper, not on
                                    <Markdown> itself — Markdown's own root sets
                                    `height: 100%; overflow: hidden`, which would
                                    fight a resize/height override applied
                                    directly to it. Markdown fills 100% of
                                    whatever height this wrapper resizes to and
                                    handles its own internal scrolling
                                    (nativeScrollbar: a plain CSS scrollbar is
                                    plenty for a reference-only preview panel). */}
                                <div class="global-bundle-machine-config-content">
                                    <Markdown
                                        text={cfg().content}
                                        scrollable={true}
                                        nativeScrollbar={true}
                                        contentClass="global-bundle-machine-config-markdown-content"
                                    />
                                </div>
                            </Show>
                        </div>
                    </div>
                )}
            </Show>

            {/* System tier — pinned above ordinary sections, always injected
                first with override wording. No move up/down: position is
                fixed server-side regardless of what a reorder call sends.
                See docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md. */}
            <div class="global-bundle-sections global-bundle-sections-system">
                <For each={model.systemSectionsAtom()}>
                    {(section) => (
                        <div
                            class="global-bundle-section global-bundle-section-system"
                            classList={{ "is-editing": model.editingSystemIdAtom() === section.id }}
                        >
                            <Show
                                when={model.editingSystemIdAtom() === section.id}
                                fallback={
                                    <div class="global-bundle-section-row">
                                        <div class="global-bundle-section-main">
                                            <span class="global-bundle-system-badge" title="AgentMux-controlled, highest priority">
                                                AgentMux
                                            </span>
                                            <span class="global-bundle-section-name">
                                                {section.name}
                                            </span>
                                        </div>
                                        <div class="global-bundle-section-actions">
                                            <button
                                                class="global-bundle-btn"
                                                onClick={() => model.startEditSystem(section)}
                                            >
                                                Edit
                                            </button>
                                            <button
                                                class="global-bundle-btn global-bundle-btn-danger"
                                                title="Delete this system entry"
                                                onClick={() => void model.removeSystem(section.id)}
                                            >
                                                Remove
                                            </button>
                                        </div>
                                    </div>
                                }
                            >
                                <SystemSectionEditor model={model} isNew={false} />
                            </Show>
                        </div>
                    )}
                </For>

                <Show when={model.editingSystemIdAtom() === NEW_SECTION_ID}>
                    <div class="global-bundle-section global-bundle-section-system is-editing">
                        <SystemSectionEditor model={model} isNew={true} />
                    </div>
                </Show>

                <Show when={model.systemSectionsAtom().length === 0 && model.editingSystemIdAtom() === null}>
                    <button
                        class="global-bundle-btn global-bundle-btn-system-add"
                        onClick={() => model.startNewSystem()}
                    >
                        + Add AgentMux system entry
                    </button>
                </Show>
            </div>

            <div class="global-bundle-sections">
                <For each={model.ordinarySectionsAtom()}>
                    {(section, i) => (
                        <div
                            class="global-bundle-section"
                            classList={{ "is-editing": model.editingIdAtom() === section.id }}
                        >
                            <Show
                                when={model.editingIdAtom() === section.id}
                                fallback={
                                    <div class="global-bundle-section-row">
                                        <div class="global-bundle-section-main">
                                            <span class="global-bundle-section-name">
                                                {section.name}
                                            </span>
                                            <Show when={section.description}>
                                                <span class="global-bundle-section-desc">
                                                    {section.description}
                                                </span>
                                            </Show>
                                        </div>
                                        <div class="global-bundle-section-actions">
                                            <button
                                                class="global-bundle-icon-btn"
                                                title="Move up"
                                                disabled={i() === 0}
                                                onClick={() => void model.move(section.id, -1)}
                                            >
                                                ↑
                                            </button>
                                            <button
                                                class="global-bundle-icon-btn"
                                                title="Move down"
                                                disabled={i() === model.ordinarySectionsAtom().length - 1}
                                                onClick={() => void model.move(section.id, 1)}
                                            >
                                                ↓
                                            </button>
                                            <button
                                                class="global-bundle-btn"
                                                onClick={() => model.startEdit(section)}
                                            >
                                                Edit
                                            </button>
                                            <button
                                                class="global-bundle-btn global-bundle-btn-danger"
                                                title="Remove from the global bundles (keeps the bundle)"
                                                onClick={() => void model.remove(section.id)}
                                            >
                                                Remove
                                            </button>
                                        </div>
                                    </div>
                                }
                            >
                                <SectionEditor model={model} isNew={false} />
                            </Show>
                        </div>
                    )}
                </For>

                {/* New-section draft renders at the END — saveEdit appends it
                    to the order, so its draft position matches where it lands. */}
                <Show when={model.editingIdAtom() === NEW_SECTION_ID}>
                    <div class="global-bundle-section is-editing">
                        <SectionEditor model={model} isNew={true} />
                    </div>
                </Show>

                <Show when={model.ordinarySectionsAtom().length === 0 && model.editingIdAtom() === null}>
                    <div class="global-bundle-empty">No global sections yet.</div>
                </Show>
            </div>

            <div class="global-bundle-add-bar">
                <button
                    class="global-bundle-btn global-bundle-btn-primary"
                    disabled={model.editingIdAtom() === NEW_SECTION_ID}
                    onClick={() => model.startNew()}
                >
                    + New section
                </button>
                <Show when={model.candidatesAtom().length > 0}>
                    <select
                        class="global-bundle-promote-select"
                        value={promoteValue()}
                        onChange={(e) => handlePromote(e.currentTarget.value)}
                    >
                        <option value="">Promote existing bundle…</option>
                        <For each={model.candidatesAtom()}>
                            {(c) => <option value={c.id}>{c.name}</option>}
                        </For>
                    </select>
                </Show>
            </div>

            {/* Same content lands in every one of these files (it doesn't
                diverge per provider) — this is visibility into WHERE it
                lands, not N separate previews. See
                docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §3.4. */}
            <div class="global-bundle-applies-to">
                <span class="global-bundle-applies-to-label">Applies to:</span>
                <For each={model.filenameGroupsAtom()}>
                    {(group) => (
                        <span
                            class="global-bundle-applies-to-chip"
                            title={group.providerNames.join(", ")}
                        >
                            <code>{group.filename}</code>
                        </span>
                    )}
                </For>
                <Show when={model.noFileProvidersAtom().length > 0}>
                    <span
                        class="global-bundle-applies-to-chip global-bundle-applies-to-chip-warning"
                        title={`${model.noFileProvidersAtom().join(", ")}: no confirmed startup-instructions file yet — see SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2`}
                    >
                        not yet applied to: {model.noFileProvidersAtom().join(", ")}
                    </span>
                </Show>
            </div>

            <div class="global-bundle-preview">
                <button
                    class="global-bundle-preview-toggle"
                    onClick={() => model.setShowPreview(!model.showPreviewAtom())}
                >
                    {model.showPreviewAtom() ? "▾" : "▸"} Combined preview
                </button>
                <Show when={model.showPreviewAtom()}>
                    {/* See the matching comment on the Claude Code provider-config
                        block above — same reason this is a wrapper div, not a
                        class applied directly to <Markdown>. */}
                    <div class="global-bundle-preview-content">
                        <Markdown
                            text={model.previewAtom() || "(empty)"}
                            scrollable={true}
                            nativeScrollbar={true}
                            contentClass="global-bundle-preview-markdown-content"
                        />
                    </div>
                </Show>
            </div>
        </div>
    );
};

GlobalBundleManager.displayName = "GlobalBundleManager";
