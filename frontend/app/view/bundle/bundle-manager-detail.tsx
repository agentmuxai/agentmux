// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The bundle manager's detail: the read-only view and the edit form with its validation report. */

import { For, Show, type Accessor, type JSX } from "solid-js";
import { Tooltip } from "@/app/element/tooltip";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { Button, TextArea, TextInput } from "@/app/element/ui";
import { PROVIDERS } from "@/app/view/agent/providers/catalog";
import { focusOnOpen } from "@/util/focusutil";
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
                    {t("bundle.form.name")}
                    <FieldHelp text={t("bundle.form.nameHelp")} />
                </span>
                <TextInput variant="plain"
                    class="bundle-view-input"
                    value={draft().name}
                    onInput={(e) => act.updateDraft("name", e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder={t("bundle.form.namePlaceholder")}
                    required
                    ref={(el) => focusOnOpen(el, { select: true })}
                />
            </label>

            <label class="bundle-view-field">
                <span class="bundle-view-field-label">
                    {t("bundle.form.description")}
                    <FieldHelp text={t("bundle.form.descriptionHelp")} />
                </span>
                <TextInput variant="plain"
                    class="bundle-view-input"
                    value={draft().description}
                    onInput={(e) =>
                        act.updateDraft("description", e.currentTarget.value)
                    }
                    onContextMenu={showTextInputContextMenu}
                    placeholder={t("bundle.form.descriptionPlaceholder")}
                />
            </label>

            {/* Who the bundle was made for: a hint, optional and
                editable. A bundle carries no harness; the agent it's
                bound to decides its own
                (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §3.2). */}
            <label class="bundle-view-field">
                <span class="bundle-view-field-label">
                    {t("bundle.form.suggestedFor")}
                    <FieldHelp text={t("bundle.form.suggestedForHelp")} />
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
                    <option value="">{t("bundle.form.anyProvider")}</option>
                    <For each={Object.values(PROVIDERS)}>
                        {(p) => <option value={p.id}>{p.displayName}</option>}
                    </For>
                </select>
            </label>
            <Show when={(PROVIDERS[draft().provider]?.supportedVendors?.length ?? 0) > 1}>
                <label class="bundle-view-field">
                    <span class="bundle-view-field-label">
                        {t("bundle.form.suggestedVendor")}
                        <FieldHelp text={t("bundle.form.suggestedVendorHelp")} />
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
                    {t("bundle.form.instructions")}
                    <FieldHelp text={t("bundle.form.instructionsHelp")} />
                </span>
                <TextArea variant="plain"
                    class="bundle-view-textarea"
                    rows={8}
                    value={draft().instructions}
                    onInput={(e) =>
                        act.updateDraft("instructions", e.currentTarget.value)
                    }
                    onContextMenu={showTextInputContextMenu}
                    placeholder={t("bundle.form.instructionsPlaceholder")}
                />
            </label>

            <label class="bundle-view-field">
                <span class="bundle-view-field-label">
                    {t("bundle.form.overrides")}
                    <FieldHelp text={t("bundle.form.overridesHelp")} />
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
                content={t("bundle.form.validateTip")}
                placement="top"
            >
                <Button
                    class="bundle-view-validate-btn"
                    onClick={act.handleValidate}
                    disabled={model.validatingAtom() || model.savingAtom() || !draft().name.trim()}
                >
                    {model.validatingAtom() ? t("bundle.form.validating") : t("bundle.form.validate")}
                </Button>
            </Tooltip>
            <Button
                class="bundle-view-cancel-btn"
                onClick={act.handleCancel}
                disabled={model.savingAtom()}
            >
                {t("bundle.form.cancel")}
            </Button>
            <Button
                type="submit"
                tone="accent"
                class="bundle-view-save-btn"
                disabled={model.savingAtom() || !draft().name.trim()}
            >
                {model.savingAtom() ? t("bundle.form.saving") : t("bundle.form.save")}
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
                        fallback={<p class="bundle-view-validation-ok">{t("bundle.form.noIssues")}</p>}
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
                {draft().id ? t("bundle.form.titleEdit") : t("bundle.form.titleNew")}
            </h2>

            <BundleIdentityFields draft={draft} act={act} />
            <BundleInstructionFields draft={draft} act={act} />
            <BundleFormActions {...props} />
            <BundleValidationReport model={model} />

            <p class="bundle-view-form-hint">
                <Rich k="bundle.form.hint" />
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
