// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GlobalBrainManager — the Armory "Memory" tab (labeled "Brain" prior to the
// PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md §4.2 naming decision — "the
// brain" is a colloquial nickname, "Memory" is the canonical term). Presents
// the workspace-wide global brain (is_global Memory bundles) as an ordered
// list of editable sections that compose into every agent's startup
// instructions file (CLAUDE.md, GEMINI.md, or similar, depending on
// provider) at launch.
//
// Context-free: owns its own GlobalBrainViewModel and drives off the
// bundle_* RPCs. Spec: specs/archive/SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md.

import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { GlobalBrainViewModel, NEW_SECTION_ID } from "./global-brain-model";
import "./global-brain.scss";

/** Inline editor card shared by the "new section" and "edit section" flows. */
function SectionEditor(props: { model: GlobalBrainViewModel; isNew: boolean }): JSX.Element {
    const { model } = props;
    return (
        <div class="global-brain-editor">
            <label class="global-brain-field">
                <span class="global-brain-field-label">Name</span>
                <input
                    class="global-brain-input"
                    type="text"
                    value={model.draftNameAtom()}
                    onInput={(e) => model.setDraftName(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="e.g. Coding Standards"
                />
            </label>
            <label class="global-brain-field">
                <span class="global-brain-field-label">Content</span>
                <textarea
                    class="global-brain-textarea"
                    rows={8}
                    value={model.draftInstructionsAtom()}
                    onInput={(e) => model.setDraftInstructions(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="Markdown injected into every agent's startup instructions file under a # [Workspace] heading."
                    spellcheck={false}
                />
            </label>
            <div class="global-brain-editor-actions">
                <button
                    class="global-brain-btn"
                    disabled={model.savingAtom()}
                    onClick={() => model.cancelEdit()}
                >
                    Cancel
                </button>
                <button
                    class="global-brain-btn global-brain-btn-primary"
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
function SystemSectionEditor(props: { model: GlobalBrainViewModel; isNew: boolean }): JSX.Element {
    const { model } = props;
    return (
        <div class="global-brain-editor global-brain-editor-system">
            <label class="global-brain-field">
                <span class="global-brain-field-label">Name</span>
                <input
                    class="global-brain-input"
                    type="text"
                    value={model.draftSystemNameAtom()}
                    onInput={(e) => model.setDraftSystemName(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="e.g. AgentMux Policy"
                />
            </label>
            <label class="global-brain-field">
                <span class="global-brain-field-label">Content</span>
                <textarea
                    class="global-brain-textarea"
                    rows={8}
                    value={model.draftSystemInstructionsAtom()}
                    onInput={(e) => model.setDraftSystemInstructions(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="Markdown injected FIRST into every agent's startup instructions file, wrapped in explicit override wording."
                    spellcheck={false}
                />
            </label>
            <div class="global-brain-editor-actions">
                <button
                    class="global-brain-btn"
                    disabled={model.savingAtom()}
                    onClick={() => model.cancelEditSystem()}
                >
                    Cancel
                </button>
                <button
                    class="global-brain-btn global-brain-btn-primary"
                    disabled={model.savingAtom() || !model.draftSystemNameAtom().trim()}
                    onClick={() => void model.saveSystemEdit()}
                >
                    {model.savingAtom() ? "Saving…" : props.isNew ? "Add system entry" : "Save"}
                </button>
            </div>
        </div>
    );
}

export const GlobalBrainManager = (): JSX.Element => {
    const model = new GlobalBrainViewModel();
    onCleanup(() => model.dispose());

    const [promoteValue, setPromoteValue] = createSignal("");

    const handlePromote = (id: string) => {
        if (!id) return;
        void model.promote(id);
        setPromoteValue("");
    };

    return (
        <div class="global-brain">
            <p class="global-brain-intro">
                Every agent inherits these sections at launch. They compose into the
                agent's startup instructions file (e.g. <code>CLAUDE.md</code>,{" "}
                <code>GEMINI.md</code>) in order, each under a{" "}
                <code># [Workspace]</code> heading.
            </p>

            <div class="global-brain-restart-note">
                Changes take effect when agents restart. Running agents keep the version
                from their last launch.
            </div>

            <Show when={model.errorAtom()}>
                <div class="global-brain-error">{model.errorAtom()}</div>
            </Show>

            {/* The CLAUDE.md a spawned Claude agent's CLAUDE_CONFIG_DIR
                actually points at by default — hand-maintained, read-only,
                not the same thing as this Global Memory tab. Shown first so
                an operator sees the full picture before the "here's where
                you'd add AgentMux's own policy" section below. codex P1, PR
                #2794: NOT the ambient ~/.claude/CLAUDE.md — that file isn't
                what a CLAUDE_CONFIG_DIR-isolated agent loads. See
                docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §5. */}
            <Show when={model.claudeGlobalConfigAtom()}>
                {(cfg) => (
                    <div class="global-brain-machine-config">
                        <div class="global-brain-machine-config-header">
                            <span
                                class="global-brain-machine-config-badge"
                                title="Hand-maintained on disk, not managed by AgentMux. Read by default-provider Claude agents (identity-bound agents use a separate, per-identity config dir not shown here)."
                            >
                                Claude Code — shared provider config
                            </span>
                            <code class="global-brain-machine-config-path">{cfg().path}</code>
                        </div>
                        <Show
                            when={cfg().exists}
                            fallback={<p class="global-brain-machine-config-empty">No file at this path yet.</p>}
                        >
                            <pre class="global-brain-machine-config-content">{cfg().content}</pre>
                        </Show>
                    </div>
                )}
            </Show>

            {/* The ambient ~/.claude/CLAUDE.md — Claude Code's own global
                config, read by a host-level CLI outside AgentMux's
                CLAUDE_CONFIG_DIR isolation (e.g. an external coding-agent
                harness). A SEPARATE block from the one above, not a
                replacement — a spawned in-app agent does NOT read this
                file. See docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §6. */}
            <Show when={model.claudeAmbientConfigAtom()}>
                {(cfg) => (
                    <div class="global-brain-machine-config">
                        <div class="global-brain-machine-config-header">
                            <span
                                class="global-brain-machine-config-badge"
                                title="Claude Code's own global config on this machine — read by a host-level Claude Code CLI running outside AgentMux's isolation. A spawned in-app agent does NOT read this file; see the shared provider config block above for what it actually reads."
                            >
                                Claude Code — host CLI config (ambient)
                            </span>
                            <code class="global-brain-machine-config-path">{cfg().path}</code>
                        </div>
                        <Show
                            when={cfg().exists}
                            fallback={<p class="global-brain-machine-config-empty">No file at this path yet.</p>}
                        >
                            <pre class="global-brain-machine-config-content">{cfg().content}</pre>
                        </Show>
                    </div>
                )}
            </Show>

            {/* System tier — pinned above ordinary sections, always injected
                first with override wording. No move up/down: position is
                fixed server-side regardless of what a reorder call sends.
                See docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md. */}
            <div class="global-brain-sections global-brain-sections-system">
                <For each={model.systemSectionsAtom()}>
                    {(section) => (
                        <div
                            class="global-brain-section global-brain-section-system"
                            classList={{ "is-editing": model.editingSystemIdAtom() === section.id }}
                        >
                            <Show
                                when={model.editingSystemIdAtom() === section.id}
                                fallback={
                                    <div class="global-brain-section-row">
                                        <div class="global-brain-section-main">
                                            <span class="global-brain-system-badge" title="AgentMux-controlled, highest priority">
                                                AgentMux
                                            </span>
                                            <span class="global-brain-section-name">
                                                {section.name}
                                            </span>
                                        </div>
                                        <div class="global-brain-section-actions">
                                            <button
                                                class="global-brain-btn"
                                                onClick={() => model.startEditSystem(section)}
                                            >
                                                Edit
                                            </button>
                                            <button
                                                class="global-brain-btn global-brain-btn-danger"
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
                    <div class="global-brain-section global-brain-section-system is-editing">
                        <SystemSectionEditor model={model} isNew={true} />
                    </div>
                </Show>

                <Show when={model.systemSectionsAtom().length === 0 && model.editingSystemIdAtom() === null}>
                    <button
                        class="global-brain-btn global-brain-btn-system-add"
                        onClick={() => model.startNewSystem()}
                    >
                        + Add AgentMux system entry
                    </button>
                </Show>
            </div>

            <div class="global-brain-sections">
                <For each={model.ordinarySectionsAtom()}>
                    {(section, i) => (
                        <div
                            class="global-brain-section"
                            classList={{ "is-editing": model.editingIdAtom() === section.id }}
                        >
                            <Show
                                when={model.editingIdAtom() === section.id}
                                fallback={
                                    <div class="global-brain-section-row">
                                        <div class="global-brain-section-main">
                                            <span class="global-brain-section-name">
                                                {section.name}
                                            </span>
                                            <Show when={section.description}>
                                                <span class="global-brain-section-desc">
                                                    {section.description}
                                                </span>
                                            </Show>
                                        </div>
                                        <div class="global-brain-section-actions">
                                            <button
                                                class="global-brain-icon-btn"
                                                title="Move up"
                                                disabled={i() === 0}
                                                onClick={() => void model.move(section.id, -1)}
                                            >
                                                ↑
                                            </button>
                                            <button
                                                class="global-brain-icon-btn"
                                                title="Move down"
                                                disabled={i() === model.ordinarySectionsAtom().length - 1}
                                                onClick={() => void model.move(section.id, 1)}
                                            >
                                                ↓
                                            </button>
                                            <button
                                                class="global-brain-btn"
                                                onClick={() => model.startEdit(section)}
                                            >
                                                Edit
                                            </button>
                                            <button
                                                class="global-brain-btn global-brain-btn-danger"
                                                title="Remove from the global brain (keeps the bundle)"
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
                    <div class="global-brain-section is-editing">
                        <SectionEditor model={model} isNew={true} />
                    </div>
                </Show>

                <Show when={model.ordinarySectionsAtom().length === 0 && model.editingIdAtom() === null}>
                    <div class="global-brain-empty">
                        No global sections yet. Add one to give every agent shared context.
                    </div>
                </Show>
            </div>

            <div class="global-brain-add-bar">
                <button
                    class="global-brain-btn global-brain-btn-primary"
                    disabled={model.editingIdAtom() === NEW_SECTION_ID}
                    onClick={() => model.startNew()}
                >
                    + New section
                </button>
                <Show when={model.candidatesAtom().length > 0}>
                    <select
                        class="global-brain-promote-select"
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
            <div class="global-brain-applies-to">
                <span class="global-brain-applies-to-label">Applies to:</span>
                <For each={model.filenameGroupsAtom()}>
                    {(group) => (
                        <span
                            class="global-brain-applies-to-chip"
                            title={group.providerNames.join(", ")}
                        >
                            <code>{group.filename}</code>
                        </span>
                    )}
                </For>
                <Show when={model.noFileProvidersAtom().length > 0}>
                    <span
                        class="global-brain-applies-to-chip global-brain-applies-to-chip-warning"
                        title={`${model.noFileProvidersAtom().join(", ")}: no confirmed startup-instructions file yet — see SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2`}
                    >
                        not yet applied to: {model.noFileProvidersAtom().join(", ")}
                    </span>
                </Show>
            </div>

            <div class="global-brain-preview">
                <button
                    class="global-brain-preview-toggle"
                    onClick={() => model.setShowPreview(!model.showPreviewAtom())}
                >
                    {model.showPreviewAtom() ? "▾" : "▸"} Combined preview (same content, every applicable file)
                </button>
                <Show when={model.showPreviewAtom()}>
                    <pre class="global-brain-preview-content">
                        {model.previewAtom() || "(no content — sections have no instructions yet)"}
                    </pre>
                </Show>
            </div>
        </div>
    );
};

GlobalBrainManager.displayName = "GlobalBrainManager";
