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
import { Modal, ModalBody, ModalFooter, ModalHeader } from "@/element/modal-v2";
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
    const [showAdvanced, setShowAdvanced] = createSignal(false);

    // Live preview of the instance slug — shown inside Advanced as
    // "Its files will live in <slug>" so power users can see what
    // their name will become. The backend stamps the real slug on
    // submit, but this is close enough for the user to recognise it.
    const hasName = () => name().trim().length > 0;

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

    // Modal v2 handles ESC (topmost-aware), backdrop click, focus
    // trap, focus restoration, and scroll lock. We only need to add
    // Enter-to-submit since the modal doesn't prescribe form semantics.
    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" && canSubmit()) {
            e.preventDefault();
            void handleSubmit();
        }
    };

    return (
        <Modal
            open={true}
            onClose={() => { if (!submitting()) props.onCancel(); }}
            closeOnBackdropClick={!submitting()}
            closeOnEscape={!submitting()}
            size="md"
        >
            <ModalHeader title={`Launch ${displayName()}`} />
            <ModalBody>
                <div class="agent-launch-modal-body" onKeyDown={handleKeyDown}>
                    <Show when={catalog()}>
                        <p class="agent-launch-modal-blurb">
                            {catalog()?.popoverMarkdown}
                        </p>
                    </Show>

                    <label class="agent-launch-modal-field">
                        <span class="agent-launch-modal-label">Give this agent a name</span>
                        <input
                            class="agent-launch-modal-input"
                            type="text"
                            maxLength={64}
                            placeholder={displayName()}
                            value={name()}
                            onInput={(e) => setName(e.currentTarget.value)}
                            disabled={submitting()}
                            aria-label="Agent name"
                        />
                        <span class="agent-launch-modal-hint">
                            So you can tell it apart from other agents. 1–64 characters.
                        </span>
                    </label>

                    <fieldset class="agent-launch-modal-field agent-launch-modal-runtime">
                        <legend class="agent-launch-modal-label">Where should it run?</legend>
                        <label class="agent-launch-modal-radio">
                            <input
                                type="radio"
                                name="agent-launch-runtime"
                                checked={runtime() === "host"}
                                onChange={() => setRuntime("host")}
                                disabled={submitting()}
                            />
                            <span>
                                <strong>On this computer</strong>
                                <span class="agent-launch-modal-hint">
                                    Fastest. The agent can read and change files on your machine.
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
                                <strong>In a safe sandbox</strong>
                                <span class="agent-launch-modal-hint">
                                    {containerSupported()
                                        ? "Slower to start, but the agent can't touch files outside its own workspace. Recommended for untrusted tasks."
                                        : "Not available for this agent."}
                                </span>
                            </span>
                        </label>
                    </fieldset>

                    <Show when={error()}>
                        <div class="agent-launch-modal-error">{error()}</div>
                    </Show>

                    <details
                        class="agent-launch-modal-advanced"
                        open={showAdvanced()}
                        onToggle={(e) => setShowAdvanced(e.currentTarget.open)}
                    >
                        <summary class="agent-launch-modal-advanced-summary">
                            Advanced options
                        </summary>
                        <div class="agent-launch-modal-advanced-body">
                            <label
                                class="agent-launch-modal-field"
                                classList={{ "agent-launch-modal-field--disabled": runtime() !== "container" }}
                            >
                                <span class="agent-launch-modal-label">Override sandbox base</span>
                                <input
                                    class="agent-launch-modal-input"
                                    type="text"
                                    placeholder={catalog()?.containerImage ?? ""}
                                    value={image()}
                                    onInput={(e) => setImage(e.currentTarget.value)}
                                    disabled={submitting() || runtime() !== "container" || !containerSupported()}
                                    aria-label="Sandbox base image"
                                />
                                <span class="agent-launch-modal-hint">
                                    {runtime() === "container"
                                        ? "Leave blank unless you know exactly which base image you need."
                                        : "Only applies to the sandbox runtime."}
                                </span>
                            </label>

                            <Show when={hasName()}>
                                <div class="agent-launch-modal-preview">
                                    <span class="agent-launch-modal-preview-label">Its files will live in</span>
                                    <code>{buildInstanceSlug(name().trim())}</code>
                                </div>
                            </Show>
                        </div>
                    </details>
                </div>
            </ModalBody>
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
