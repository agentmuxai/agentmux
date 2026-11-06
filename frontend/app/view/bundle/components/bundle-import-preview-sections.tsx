// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The import preview's smaller sections: the name, the instructions, context files, MCP servers, account requirements and warnings. */

import { For, Show, type JSX } from "solid-js";

import { Button as UiButton, IconButton, TextInput } from "@/app/element/ui";
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
            <span class="bundle-import-field-label">Bundle name</span>
            <TextInput variant="plain"
                class="bundle-import-input"
                value={s.bundleName()}
                onInput={(e) => s.setBundleName(e.currentTarget.value)}
            />
            <Show when={s.preview.name_collision}>
                <div class="bundle-import-hint">
                    A bundle named this already exists.{" "}
                    <UiButton
                        variant="link"
                        tone="accent"
                        onClick={() => s.setBundleName(act.suggestedAltName())}
                    >
                        Use "{act.suggestedAltName()}" instead
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
                <span>Include instructions</span>
            </label>
            <Show when={s.includeInstructions() && preview.instructions_preview}>
                <div class="bundle-import-instructions-preview">
                    <UiButton
                        variant="link"
                        tone="accent"
                        onClick={() => s.setInstructionsExpanded((v) => !v)}
                    >
                        {s.instructionsExpanded() ? "Hide" : "Show"} preview
                    </UiButton>
                    <Show when={s.instructionsExpanded()}>
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
    );
}

export function ContextSection(props: SectionProps): JSX.Element {
    const { s, act } = props;
    return (
        <Show when={s.preview.context_files.length > 0}>
            <section class="bundle-import-section">
                <h3 class="bundle-import-section-title">Context files</h3>
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
                <h3 class="bundle-import-section-title">Not imported: MCP servers</h3>
                <div class="bundle-import-hint">
                    Bundles don't carry MCP servers. Add these in Connectors and bind them to
                    the agents that need them.
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
                <h3 class="bundle-import-section-title">Account requirements</h3>
                <p class="bundle-import-requirements-summary">
                    Depends on {preview.requirements.length} account(s):{" "}
                    {preview.requirements
                        .map((r) => `${r.provider} (${r.resolved ? "resolved" : "not connected"})`)
                        .join(", ")}
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
                <IconButton
                    icon="xmark"
                    label="Dismiss warnings"
                    tooltip={false}
                    density="compact"
                    class="bundle-import-warnings-dismiss"
                    onClick={() => s.setWarningsDismissed(true)}
                />
                <For each={s.preview.warnings}>{(w) => <div class="bundle-import-warning-line">{w}</div>}</For>
                <Show when={s.preview.warnings_truncated}>
                    <div class="bundle-import-warning-line">…more warnings not shown</div>
                </Show>
            </div>
        </Show>
    );
}
