// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { For, Show } from "solid-js";
import type { ForgeViewModel } from "../forge-model";
import { PROVIDERS } from "../forge-constants";

export function ForgeForm(props: { model: ForgeViewModel }): JSX.Element {
    const view = props.model.viewAtom;
    const editingAgent = props.model.editingAgentAtom;
    const loading = props.model.loadingAtom;
    const error = props.model.errorAtom;

    const isEdit = () => view() === "edit" && editingAgent() != null;

    const [name, setName] = createSignal(editingAgent()?.name ?? "");
    const [icon, setIcon] = createSignal(editingAgent()?.icon ?? "\u2726");
    const [provider, setProvider] = createSignal(editingAgent()?.provider ?? "claude");
    const [description, setDescription] = createSignal(editingAgent()?.description ?? "");
    const [workingDirectory, setWorkingDirectory] = createSignal(editingAgent()?.working_directory ?? "");
    const [shell, setShell] = createSignal(editingAgent()?.shell ?? "");
    const [providerFlags, setProviderFlags] = createSignal(editingAgent()?.provider_flags ?? "");
    const [autoStart, setAutoStart] = createSignal(editingAgent()?.auto_start === 1);

    const handleSubmit = async (e: Event) => {
        e.preventDefault();
        if (!name().trim()) return;
        if (isEdit()) {
            await props.model.updateAgent({
                id: editingAgent()!.id,
                name: name().trim(),
                icon: icon() || "\u2726",
                provider: provider(),
                description: description(),
                working_directory: workingDirectory(),
                shell: shell(),
                provider_flags: providerFlags(),
                auto_start: autoStart() ? 1 : 0,
            });
        } else {
            await props.model.createAgent({
                name: name().trim(),
                icon: icon() || "\u2726",
                provider: provider(),
                description: description(),
                working_directory: workingDirectory(),
                shell: shell(),
                provider_flags: providerFlags(),
                auto_start: autoStart() ? 1 : 0,
            });
        }
    };

    const title = () => isEdit() ? "Edit Agent" : "New Agent";

    return (
        <div class="forge-pane">
            <div class="forge-header">
                <span class="forge-title">
                    Forge&nbsp;/&nbsp;<span class="forge-title-sub">{title()}</span>
                </span>
            </div>
            <div class="forge-divider" />
            <form class="forge-form" onSubmit={handleSubmit}>
                <div class="forge-form-row">
                    <label class="forge-form-label">Icon</label>
                    <input
                        class="forge-form-input forge-form-input-icon"
                        value={icon()}
                        maxLength={4}
                        onInput={(e) => setIcon(e.currentTarget.value)}
                        placeholder="\u2726"
                    />
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label">Name</label>
                    <input
                        class="forge-form-input"
                        value={name()}
                        onInput={(e) => setName(e.currentTarget.value)}
                        placeholder="My Agent"
                        autofocus
                        required
                    />
                </div>

                <div class="forge-form-row forge-form-row-col">
                    <label class="forge-form-label">Provider</label>
                    <div class="forge-form-providers">
                        <For each={PROVIDERS}>{(p) =>
                            <label class="forge-form-provider-opt">
                                <input
                                    type="radio"
                                    name="provider"
                                    value={p.id}
                                    checked={provider() === p.id}
                                    onInput={() => setProvider(p.id)}
                                />
                                <span class="forge-form-provider-label">{p.label}</span>
                                <span class="forge-form-provider-cmd">{p.cmd}</span>
                            </label>
                        }</For>
                    </div>
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label">Description</label>
                    <input
                        class="forge-form-input"
                        value={description()}
                        onInput={(e) => setDescription(e.currentTarget.value)}
                        placeholder="Optional description"
                    />
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label">Working Directory</label>
                    <input
                        class="forge-form-input"
                        value={workingDirectory()}
                        onInput={(e) => setWorkingDirectory(e.currentTarget.value)}
                        placeholder="e.g. ~/.agentmux/agents/myagent"
                    />
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label">Shell</label>
                    <select
                        class="forge-form-input forge-form-select"
                        value={shell()}
                        onInput={(e) => setShell(e.currentTarget.value)}
                    >
                        <option value="">Default</option>
                        <option value="bash">bash</option>
                        <option value="pwsh">pwsh</option>
                        <option value="cmd">cmd</option>
                        <option value="zsh">zsh</option>
                    </select>
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label">Provider Flags</label>
                    <input
                        class="forge-form-input"
                        value={providerFlags()}
                        onInput={(e) => setProviderFlags(e.currentTarget.value)}
                        placeholder="Extra CLI arguments"
                    />
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label forge-form-checkbox-label">
                        <input
                            type="checkbox"
                            checked={autoStart()}
                            onInput={(e) => setAutoStart(e.currentTarget.checked)}
                        />
                        Auto Start
                    </label>
                </div>

                <Show when={error()}>
                    <div class="forge-form-error">{error()}</div>
                </Show>

                <div class="forge-form-actions">
                    <button type="submit" class="forge-btn-primary" disabled={loading() || !name().trim()}>
                        {loading() ? "Saving\u2026" : "Save"}
                    </button>
                    <button type="button" class="forge-btn-secondary" onClick={() => props.model.cancelForm()}>
                        Cancel
                    </button>
                </div>
            </form>
        </div>
    );
}
