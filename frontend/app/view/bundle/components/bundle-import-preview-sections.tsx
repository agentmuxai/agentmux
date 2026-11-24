// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The import preview's smaller sections: the name, the instructions, context files, MCP servers, account requirements and warnings. */

import { For, Show, type JSX } from "solid-js";

import { Button as UiButton, CloseButton, TextInput } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { formatBytes } from "@/util/format-bytes";
import type { BundleImportPreviewActions, BundleImportPreviewState } from "./bundle-import-preview-state";

interface SectionProps {
    s: BundleImportPreviewState;
    act: BundleImportPreviewActions;
}

export function NameField(props: SectionProps): JSX.Element {
    const { s, act } = props;
    return (
        <label class="bundle-import-field">
            <span class="bundle-import-field-label">{t("bundle.import.bundleName")}</span>
            <TextInput variant="plain"
                class="bundle-import-input"
                value={s.bundleName()}
                onInput={(e) => s.setBundleName(e.currentTarget.value)}
            />
            <Show when={s.preview.name_collision}>
                <div class="bundle-import-hint">
                    {t("bundle.import.bundleExists")}{" "}
                    <UiButton
                        variant="link"
                        tone="accent"
                        onClick={() => s.setBundleName(act.suggestedAltName())}
                    >
                        {t("bundle.import.useInstead", { name: act.suggestedAltName() })}
                    </UiButton>
                </div>
            </Show>
        </label>
    );
}

export function InstructionsSection(props: SectionProps): JSX.Element {
    const { s } = props;
    const preview = s.preview;
    return (
        <section class="bundle-import-section">
            <label class="bundle-import-checkbox-row">
                <input
                    type="checkbox"
                    checked={s.includeInstructions()}
                    onChange={(e) => s.setIncludeInstructions(e.currentTarget.checked)}
                />
                <span>{t("bundle.import.includeInstructions")}</span>
            </label>
            <Show when={s.includeInstructions() && preview.instructions_preview}>
                <div class="bundle-import-instructions-preview">
                    <UiButton
                        variant="link"
                        tone="accent"
                        onClick={() => s.setInstructionsExpanded((v) => !v)}
                    >
                        {s.instructionsExpanded() ? t("bundle.import.hidePreview") : t("bundle.import.showPreview")}
                    </UiButton>
                    <Show when={s.instructionsExpanded()}>
                        <pre class="bundle-import-instructions-text">{preview.instructions_preview}</pre>
                        <Show when={preview.instructions_truncated}>
                            <div class="bundle-import-hint">
                                {t("bundle.import.truncated", { total: preview.instructions_total_chars })}
                            </div>
                        </Show>
                    </Show>
                </div>
            </Show>
        </section>
    );
}

export function ContextSection(props: SectionProps): JSX.Element {
    const { s, act } = props;
    return (
        <Show when={s.preview.context_files.length > 0}>
            <section class="bundle-import-section">
                <h3 class="bundle-import-section-title">{t("bundle.import.contextFiles")}</h3>
                <For each={s.preview.context_files}>
                    {(cf) => (
                        <label class="bundle-import-checkbox-row">
                            <input
                                type="checkbox"
                                checked={!!s.contextChecked()[cf.id]}
                                onChange={() => act.toggleContext(cf.id)}
                            />
                            <span class="bundle-import-item-name">{cf.display_path}</span>
                            <span class="bundle-import-item-meta">{formatBytes(cf.size_bytes)}</span>
                        </label>
                    )}
                </For>
            </section>
        </Show>
    );
}

export function McpSection(props: SectionProps): JSX.Element {
    const preview = props.s.preview;
    // Listed, not offered: a bundle carries no MCP servers
    // (SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md §3.1).
    return (
        <Show when={preview.mcp_servers.length > 0}>
            <section class="bundle-import-section">
                <h3 class="bundle-import-section-title">{t("bundle.import.mcpTitle")}</h3>
                <div class="bundle-import-hint">
                    {t("bundle.import.mcpNote")}
                </div>
                <For each={preview.mcp_servers}>
                    {(m) => (
                        <div class="bundle-import-checkbox-row">
                            <span class="bundle-import-item-name">
                                {m.display.name ?? m.source_path.split("/").pop()}
                            </span>
                            <Show when={m.display.command}>
                                <span class="bundle-import-item-meta">{m.display.command}</span>
                            </Show>
                        </div>
                    )}
                </For>
            </section>
        </Show>
    );
}

export function RequirementsSection(props: SectionProps): JSX.Element {
    const preview = props.s.preview;
    return (
        <Show when={preview.requirements.length > 0}>
            <section class="bundle-import-section">
                <h3 class="bundle-import-section-title">{t("bundle.import.requirements")}</h3>
                <p class="bundle-import-requirements-summary">
                    {t("bundle.import.dependsOn", {
                        n: preview.requirements.length,
                        list: preview.requirements
                            .map((r) => `${r.provider} (${r.resolved ? t("bundle.import.resolved") : t("bundle.import.notConnected")})`)
                            .join(", "),
                    })}
                </p>
            </section>
        </Show>
    );
}

export function WarningsBanner(props: SectionProps): JSX.Element {
    const { s } = props;
    return (
        <Show when={s.preview.warnings.length > 0 && !s.warningsDismissed()}>
            <div class="bundle-import-warnings-banner">
                <CloseButton
                    size="sm"
                    label={t("bundle.import.dismissWarnings")}
                    tooltip={false}
                    class="bundle-import-warnings-dismiss"
                    onClick={() => s.setWarningsDismissed(true)}
                />
                <For each={s.preview.warnings}>{(w) => <div class="bundle-import-warning-line">{w}</div>}</For>
                <Show when={s.preview.warnings_truncated}>
                    <div class="bundle-import-warning-line">{t("bundle.import.moreWarnings")}</div>
                </Show>
            </div>
        </Show>
    );
}
