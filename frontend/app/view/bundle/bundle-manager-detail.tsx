// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The bundle manager's detail: the read-only view and the edit form with its validation report. */

import { For, Show, type Accessor, type JSX } from "solid-js";
import { Tooltip } from "@/app/element/tooltip";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { Button, TextArea, TextInput } from "@/app/element/ui";
import { PROVIDERS } from "@/app/view/agent/providers/catalog";
import { focusOnOpen } from "@/util/focusutil";
import type { Bundle } from "@/app/store/rpc-api";
import type { BundleActions } from "./bundle-manager-actions";
import type { BundleDraft, BundleViewModel } from "./bundle-model";
import { BundleProviderInstructionsSection } from "./BundleProviderInstructionsSection";
import { BundleReadOnly } from "./bundle-manager-readonly";

interface DetailProps {
    model: BundleViewModel;
    act: BundleActions;
}

/** Small (i) icon with a hover tooltip — contextual in-line documentation
 *  next to a field label, explaining what it does in ABF terms without
 *  requiring a trip to the docs. */
const FieldHelp = (props: { text: string }): JSX.Element => (
    <Tooltip content={props.text} placement="right">
        <i class="fa-solid fa-circle-info bundle-view-field-help" aria-hidden="true" />
    </Tooltip>
);

/** Name, description and the provider hint (with its vendor, when the provider has several). */
function BundleIdentityFields(props: { draft: Accessor<BundleDraft>; act: BundleActions }): JSX.Element {
    const { draft, act } = props;
    return (
        <>
            <label class="bundle-view-field">
                <span class="bundle-view-field-label">
                    Name *
                    <FieldHelp text="The bundle's display name — shown in Memory → Bundles and in the agent launch picker. Required." />
                </span>
                <TextInput variant="plain"
                    class="bundle-view-input"
                    value={draft().name}
                    onInput={(e) => act.updateDraft("name", e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="e.g. Claude-coder"
                    required
                    ref={(el) => focusOnOpen(el, { select: true })}
                />
            </label>

            <label class="bundle-view-field">
                <span class="bundle-view-field-label">
                    Description
                    <FieldHelp text="Optional short label shown under the bundle's name in the launch picker. Purely cosmetic — has no effect on what gets injected into the agent." />
                </span>
                <TextInput variant="plain"
                    class="bundle-view-input"
                    value={draft().description}
                    onInput={(e) =>
                        act.updateDraft("description", e.currentTarget.value)
                    }
                    onContextMenu={showTextInputContextMenu}
                    placeholder="Short label shown in the launch picker"
                />
            </label>

            {/* Who the bundle was made for: a hint, optional and
                editable. A bundle carries no harness; the agent it's
                bound to decides its own
                (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §3.2). */}
            <label class="bundle-view-field">
                <span class="bundle-view-field-label">
                    Suggested for
                    <FieldHelp text="The provider this bundle was made with in mind. A hint only: a bundle can be bound to any agent, and the agent's own provider decides which CLI runs." />
                </span>
                <select
                    class="bundle-view-input"
                    value={draft().provider}
                    onChange={(e) => {
                        const provider = e.currentTarget.value;
                        act.updateDraft("provider", provider);
                        act.updateDraft("model", PROVIDERS[provider]?.supportedVendors?.[0] ?? "");
                    }}
                >
                    <option value="">Any provider</option>
                    <For each={Object.values(PROVIDERS)}>
                        {(p) => <option value={p.id}>{p.displayName}</option>}
                    </For>
                </select>
            </label>
            <Show when={(PROVIDERS[draft().provider]?.supportedVendors?.length ?? 0) > 1}>
                <label class="bundle-view-field">
                    <span class="bundle-view-field-label">
                        Suggested vendor
                        <FieldHelp text="The backend that provider was meant to talk to. A hint, like the provider." />
                    </span>
                    <select
                        class="bundle-view-input"
                        value={draft().model}
                        onChange={(e) => act.updateDraft("model", e.currentTarget.value)}
                    >
                        <For each={PROVIDERS[draft().provider]?.supportedVendors ?? []}>
                            {(v) => <option value={v}>{v}</option>}
                        </For>
                    </select>
                </label>
            </Show>
        </>
    );
}

/** The default instructions and the per-provider overrides. */
function BundleInstructionFields(props: { draft: Accessor<BundleDraft>; act: BundleActions }): JSX.Element {
    const { draft, act } = props;
    return (
        <>
            <label class="bundle-view-field">
                <span class="bundle-view-field-label">
                    Instructions
                    <FieldHelp text="The default system prompt injected into the agent's context at launch — provider-agnostic, applies regardless of which CLI/harness the agent uses. To override it for one specific harness, add a per-provider variant below; the default still applies to every provider without one." />
                </span>
                <TextArea variant="plain"
                    class="bundle-view-textarea"
                    rows={8}
                    value={draft().instructions}
                    onInput={(e) =>
                        act.updateDraft("instructions", e.currentTarget.value)
                    }
                    onContextMenu={showTextInputContextMenu}
                    placeholder="System prompt. The agent's soul."
                />
            </label>

            <label class="bundle-view-field">
                <span class="bundle-view-field-label">
                    Per-provider instruction overrides
                    <FieldHelp text="ABF v0.2 §2.2. Each entry replaces the Instructions above when this bundle runs on that provider — it is an override, not an addition, and providers without an entry use the default unchanged. On export each becomes instructions/<provider>/AGENTS.md, so a key that is not a usable directory name is skipped; this form flags those before you save." />
                </span>
                <BundleProviderInstructionsSection
                    value={draft().instructions_by_provider}
                    onChange={(next) =>
                        act.updateDraft("instructions_by_provider", next)
                    }
                />
            </label>
        </>
    );
}

function BundleFormActions(props: DetailProps & { draft: Accessor<BundleDraft> }): JSX.Element {
    const { model, act, draft } = props;
    return (
        <div class="bundle-view-form-actions">
            <Tooltip
                content="Structurally checks this draft: unknown provider keys, unsafe or colliding context-file paths, and malformed JSON in the fields not yet editable here. Advisory only — never blocks Save."
                placement="top"
            >
                <Button
                    class="bundle-view-validate-btn"
                    onClick={act.handleValidate}
                    disabled={model.validatingAtom() || model.savingAtom() || !draft().name.trim()}
                >
                    {model.validatingAtom() ? "Validating…" : "Validate"}
                </Button>
            </Tooltip>
            <Button
                class="bundle-view-cancel-btn"
                onClick={act.handleCancel}
                disabled={model.savingAtom()}
            >
                Cancel
            </Button>
            <Button
                type="submit"
                tone="accent"
                class="bundle-view-save-btn"
                disabled={model.savingAtom() || !draft().name.trim()}
            >
                {model.savingAtom() ? "Saving…" : "Save"}
            </Button>
        </div>
    );
}

/** Structural ABF validation results (bundle.validate) — advisory only, never blocks Save. Cleared on any
 *  further edit (updateDraft) or draft replacement so a stale report never lingers onto different content. */
function BundleValidationReport(props: { model: BundleViewModel }): JSX.Element {
    return (
        <Show when={props.model.validationAtom()}>
            {(report) => (
                <div
                    class="bundle-view-validation"
                    classList={{ "is-valid": report().is_valid }}
                >
                    <Show
                        when={report().issues.length > 0}
                        fallback={<p class="bundle-view-validation-ok">No structural issues found.</p>}
                    >
                        <ul class="bundle-view-validation-list">
                            <For each={report().issues}>
                                {(issue) => (
                                    <li
                                        class="bundle-view-validation-item"
                                        classList={{
                                            "is-error": issue.severity === "error",
                                            "is-warning": issue.severity === "warning",
                                        }}
                                    >
                                        <span class="bundle-view-validation-field">{issue.field}</span>
                                        {": "}
                                        {issue.message}
                                    </li>
                                )}
                            </For>
                        </ul>
                    </Show>
                </div>
            )}
        </Show>
    );
}

function BundleForm(props: DetailProps & { draft: Accessor<BundleDraft> }): JSX.Element {
    const { model, act, draft } = props;
    return (
        <form
            class="bundle-view-form"
            onSubmit={(e) => {
                e.preventDefault();
                act.handleSave();
            }}
        >
            <h2 class="bundle-view-form-title">
                {draft().id ? "Edit Bundle" : "New Bundle"}
            </h2>

            <BundleIdentityFields draft={draft} act={act} />
            <BundleInstructionFields draft={draft} act={act} />
            <BundleFormActions {...props} />
            <BundleValidationReport model={model} />

            <p class="bundle-view-form-hint">
                Skills are managed on the bundle's own detail view (Cancel to
                get back there), not in this edit form — they take effect live
                and don't need a Save. MCP servers aren't part of a bundle: bind
                them to agents in Connectors. Context files are still
                persisted as JSON and round-trip cleanly through this form; use{" "}
                <strong>Validate</strong> above to catch unsafe paths or malformed JSON in
                them. To bring in an existing <code>.abf</code> archive's components directly,
                use <strong>Import Bundle</strong> from the bundle list.
            </p>
        </form>
    );
}

export function BundleDetail(props: DetailProps): JSX.Element {
    const { model, act } = props;
    return (
        <div class="bundle-view-detail">
            <act.confirm.Dialog />
            <Show when={model.errorAtom()}>
                <div class="bundle-view-error">{model.errorAtom()}</div>
            </Show>

            <Show
                when={model.draftAtom()}
                fallback={
                    <Show when={model.selectedAtom()}>
                        {(memory) => <BundleReadOnly model={model} act={act} memory={memory} />}
                    </Show>
                }
            >
                {(draft) => <BundleForm model={model} act={act} draft={draft} />}
            </Show>
        </div>
    );
}
