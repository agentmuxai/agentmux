// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentNewMemoryModalPanel — creates a new Memory bundle from the
 * Launch modal's "+ New" affordance.
 *
 * Memory bundles are organized text content (notes, instructions,
 * project context files). The modal supports three seed modes:
 *   - Empty: just create the bundle; user adds content later from
 *     the Memory pane.
 *   - Paste text: pastes go into a single `notes.md` context file.
 *   - Pick files: deferred (file picker integration is its own work).
 *
 * The actual UpsertMemory RPC lives in ModalLayer's new-memory
 * dispatch (mirrors the Launch modal's onSubmit pattern) so the
 * layer's `submitting()` flag — which gates safeClose — tracks the
 * in-flight call. Reagent P1 on PR #911.
 *
 * Phase γ of SPEC_LAUNCH_MODAL_PROFILE_SECTION_2026_05_18.md.
 */

import { Show, createSignal, type JSX } from "solid-js";

import { Button } from "@/element/button";

type SeedMode = "empty" | "paste" | "files";

interface NewMemoryFormData {
    name: string;
    description: string;
    /** JSON-encoded array of `{ path, content }` — server stores this
     *  verbatim in the Memory row's context_files column. Empty array
     *  ("[]") for the empty / pick-files seed modes. */
    contextFiles: string;
}

interface AgentNewMemoryModalPanelProps {
    initialName?: string;
    onCancel: () => void;
    onSubmit: (formData: NewMemoryFormData) => Promise<void>;
}

export const AgentNewMemoryModalPanel = (
    props: AgentNewMemoryModalPanelProps,
): JSX.Element => {
    const [name, setName] = createSignal(props.initialName ?? "");
    const [description, setDescription] = createSignal("");
    const [seedMode, setSeedMode] = createSignal<SeedMode>("empty");
    const [pasteText, setPasteText] = createSignal("");
    const [submitting, setSubmitting] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    const canSubmit = () => name().trim().length > 0 && !submitting();

    const submit = async () => {
        if (!canSubmit()) return;
        setSubmitting(true);
        setError(null);
        try {
            const contextFiles =
                seedMode() === "paste" && pasteText().trim().length > 0
                    ? JSON.stringify([
                          { path: "notes.md", content: pasteText() },
                      ])
                    : "[]";
            await props.onSubmit({
                name: name().trim(),
                description: description().trim(),
                contextFiles,
            });
            setSubmitting(false);
        } catch (e) {
            setError((e as Error)?.message ?? String(e));
            setSubmitting(false);
        }
    };

    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void submit();
        }
        // Escape handled by the layer's overlay handler so the
        // layer-level submitting() gate takes effect uniformly
        // across all focusable elements — reagent P2 on PR #911.
    };

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">New Bundle</h2>
                <p class="modal-panel-description">
                    A bundle holds text the agent reads at launch — notes,
                    project context, style guides, instructions.
                </p>
            </header>
            <div class="modal-panel-body agent-new-bundle-modal-body">
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Name</span>
                    <input
                        type="text"
                        class="agent-new-bundle-modal-input"
                        autofocus
                        placeholder="Project Apollo notes, Style guide, …"
                        value={name()}
                        onInput={(e) => setName(e.currentTarget.value)}
                        onKeyDown={onKeyDown}
                        disabled={submitting()}
                    />
                </label>
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">
                        Description <span class="agent-new-bundle-modal-optional">(optional)</span>
                    </span>
                    <input
                        type="text"
                        class="agent-new-bundle-modal-input"
                        placeholder="What's this bundle for?"
                        value={description()}
                        onInput={(e) => setDescription(e.currentTarget.value)}
                        onKeyDown={onKeyDown}
                        disabled={submitting()}
                    />
                </label>
                <fieldset class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Seed content</span>
                    <div class="agent-new-bundle-modal-radios">
                        <label class="agent-new-bundle-modal-radio">
                            <input
                                type="radio"
                                name="seed-mode"
                                checked={seedMode() === "empty"}
                                onChange={() => setSeedMode("empty")}
                                disabled={submitting()}
                            />
                            <span>Start empty — add files later from the Armory (ABF)</span>
                        </label>
                        <label class="agent-new-bundle-modal-radio">
                            <input
                                type="radio"
                                name="seed-mode"
                                checked={seedMode() === "paste"}
                                onChange={() => setSeedMode("paste")}
                                disabled={submitting()}
                            />
                            <span>Paste text now (saved as <code>notes.md</code>)</span>
                        </label>
                        <label class="agent-new-bundle-modal-radio" title="Coming soon">
                            <input
                                type="radio"
                                name="seed-mode"
                                disabled
                            />
                            <span style={{ "opacity": 0.6 }}>
                                Pick files from disk <em>(coming soon)</em>
                            </span>
                        </label>
                    </div>
                </fieldset>
                <Show when={seedMode() === "paste"}>
                    <label class="agent-new-bundle-modal-field">
                        <span class="agent-new-bundle-modal-label">Notes</span>
                        <textarea
                            class="agent-new-bundle-modal-textarea"
                            rows={8}
                            placeholder="Paste any text the agent should remember…"
                            value={pasteText()}
                            onInput={(e) => setPasteText(e.currentTarget.value)}
                            disabled={submitting()}
                        />
                    </label>
                </Show>
                {error() && (
                    <div class="agent-new-bundle-modal-error">{error()}</div>
                )}
            </div>
            <footer class="modal-panel-footer">
                <Button onClick={() => props.onCancel()} disabled={submitting()} data-modal-dismiss>
                    Cancel
                </Button>
                <Button
                    onClick={() => void submit()}
                    className="green solid"
                    disabled={!canSubmit()}
                >
                    {submitting() ? "Creating…" : "Create"}
                </Button>
            </footer>
        </>
    );
};

AgentNewMemoryModalPanel.displayName = "AgentNewMemoryModalPanel";
