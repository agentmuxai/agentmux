// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentLaunchModal — shown when the user clicks a definition card in
 * the agent picker. Collects the instance name + runtime (host vs
 * container) and submits them to the caller, which is responsible
 * for calling launchForgeAgent with the overrides.
 *
 * See docs/specs/SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md §6.
 */

import { createMemo, createSignal, Show, type JSX } from "solid-js";
import { Button } from "@/element/button";
import { Modal, ModalContent, ModalFooter, ModalHeader } from "@/element/modal";
import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { buildInstanceSlug, slugifyInstanceName } from "../defaults/instance-slug";

export interface LaunchOverrides {
    /** Instance name — written into AGENTMUX_AGENT_ID and used to
     *  derive the working directory. */
    instanceName: string;
    /** "host" runs directly on the OS. "container" runs inside
     *  Docker/Podman. */
    agentType: "host" | "container";
    /** "local" pairs with "host"; "docker" pairs with "container". */
    environment: "local" | "docker";
    /** Only set when agentType === "container". */
    containerImage?: string;
}

interface AgentLaunchModalProps {
    agent: ForgeAgent;
    onCancel: () => void;
    onSubmit: (overrides: LaunchOverrides) => Promise<void> | void;
}

export const AgentLaunchModal = (props: AgentLaunchModalProps): JSX.Element => {
    const catalog = createMemo(() => getCliCatalogEntry(props.agent.provider));
    const displayName = () => catalog()?.displayName ?? props.agent.name;

    const [name, setName] = createSignal("");
    const [runtime, setRuntime] = createSignal<"host" | "container">("host");
    const [image, setImage] = createSignal<string>("");
    const [submitting, setSubmitting] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    // Live preview — the backend computes the real stamp on submit,
    // but showing something now reassures the user that their name
    // will become a directory.
    const previewDir = createMemo(() => {
        const trimmed = name().trim();
        if (!trimmed) return "";
        return `data/agents/${buildInstanceSlug(trimmed)}/`;
    });

    const canSubmit = () => !submitting() && slugifyInstanceName(name()).length > 0;

    const containerSupported = () => catalog()?.containerSupported ?? true;

    const resolvedImage = () => {
        const v = image().trim();
        if (v) return v;
        return catalog()?.containerImage ?? "";
    };

    const handleSubmit = async () => {
        if (!canSubmit()) return;
        setSubmitting(true);
        setError(null);
        try {
            await props.onSubmit({
                instanceName: name().trim(),
                agentType: runtime(),
                environment: runtime() === "container" ? "docker" : "local",
                containerImage: runtime() === "container" ? resolvedImage() : undefined,
            });
        } catch (e: any) {
            setError(String(e?.message ?? e));
            setSubmitting(false);
        }
        // Success: parent closes the modal so we leave submitting true.
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" && canSubmit()) {
            e.preventDefault();
            void handleSubmit();
        } else if (e.key === "Escape") {
            e.preventDefault();
            props.onCancel();
        }
    };

    return (
        <Modal onClickOut={() => { if (!submitting()) props.onCancel(); }}>
            <ModalHeader title={`Launch ${displayName()}`} />
            <ModalContent>
                <div class="agent-launch-modal-body" onKeyDown={handleKeyDown}>
                    <Show when={catalog()}>
                        <p class="agent-launch-modal-blurb">
                            {catalog()?.popoverMarkdown}
                        </p>
                    </Show>

                    <label class="agent-launch-modal-field">
                        <span class="agent-launch-modal-label">Name</span>
                        <input
                            class="agent-launch-modal-input"
                            type="text"
                            maxLength={64}
                            placeholder={displayName()}
                            value={name()}
                            onInput={(e) => setName(e.currentTarget.value)}
                            disabled={submitting()}
                            ref={(el) => setTimeout(() => el?.focus(), 0)}
                            aria-label="Instance name"
                        />
                        <span class="agent-launch-modal-hint">
                            1–64 characters. Becomes part of the working directory.
                        </span>
                    </label>

                    <fieldset class="agent-launch-modal-field agent-launch-modal-runtime">
                        <legend class="agent-launch-modal-label">Runtime</legend>
                        <label class="agent-launch-modal-radio">
                            <input
                                type="radio"
                                name="agent-launch-runtime"
                                checked={runtime() === "host"}
                                onChange={() => setRuntime("host")}
                                disabled={submitting()}
                            />
                            <span>
                                <strong>Host</strong>
                                <span class="agent-launch-modal-hint">
                                    Runs on your machine with your shell.
                                </span>
                            </span>
                        </label>
                        <label
                            class="agent-launch-modal-radio"
                            classList={{ "agent-launch-modal-radio--disabled": !containerSupported() }}
                        >
                            <input
                                type="radio"
                                name="agent-launch-runtime"
                                checked={runtime() === "container"}
                                onChange={() => setRuntime("container")}
                                disabled={submitting() || !containerSupported()}
                            />
                            <span>
                                <strong>Container</strong>
                                <span class="agent-launch-modal-hint">
                                    {containerSupported()
                                        ? "Runs inside Docker/Podman — sandboxed."
                                        : "Not supported for this CLI."}
                                </span>
                            </span>
                        </label>
                    </fieldset>

                    <Show when={runtime() === "container" && containerSupported()}>
                        <label class="agent-launch-modal-field">
                            <span class="agent-launch-modal-label">Image</span>
                            <input
                                class="agent-launch-modal-input"
                                type="text"
                                placeholder={catalog()?.containerImage ?? ""}
                                value={image()}
                                onInput={(e) => setImage(e.currentTarget.value)}
                                disabled={submitting()}
                                aria-label="Container image"
                            />
                            <span class="agent-launch-modal-hint">
                                Leave blank to use the default image.
                            </span>
                        </label>
                    </Show>

                    <Show when={previewDir()}>
                        <div class="agent-launch-modal-preview">
                            <span class="agent-launch-modal-preview-label">Working dir:</span>
                            <code>{previewDir()}</code>
                        </div>
                    </Show>

                    <Show when={error()}>
                        <div class="agent-launch-modal-error">{error()}</div>
                    </Show>
                </div>
            </ModalContent>
            <ModalFooter>
                <Button onClick={props.onCancel} disabled={submitting()}>
                    Cancel
                </Button>
                <Button onClick={() => void handleSubmit()} disabled={!canSubmit()}>
                    {submitting() ? "Launching…" : "Launch"}
                </Button>
            </ModalFooter>
        </Modal>
    );
};

AgentLaunchModal.displayName = "AgentLaunchModal";
