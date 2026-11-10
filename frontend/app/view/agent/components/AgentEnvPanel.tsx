// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentEnvPanel — the "Environment…" modal, opened from My Agents' row
 * menu: the extra environment variables
 * a definition launches with, one `NAME=VALUE` per line. Stored as the
 * definition's `env` content (`getagentcontent` / `setagentcontent`) and read
 * at launch by the same parser the editor shows its ignored lines with
 * (agent-env.ts). An agent can change it too (`Agents env_set`); the panel
 * reloads when that happens, unless there are unsaved edits.
 */

import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import { Button, Field, TextArea } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ENV_CONTENT_TYPE, parseEnvText } from "../agent-env";
import { EndpointNoticeLine } from "./EndpointNoticeLine";
import "./AgentEnvPanel.scss";
import "./AgentPrimitiveModal.scss";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

export const envHint = (): string => t("agentEnv.hint");

/**
 * What the environment box holds. A panel that gets its own from the list keeps its edits when the list
 * rebuilds it (a resize that moves the panel to another row remounts it); one that is given none makes its own.
 */
export const createEnvDraft = () => {
    /** The text as stored; `draft` is what the box holds. */
    const [stored, setStored] = createSignal("");
    const [draft, setDraft] = createSignal("");
    const [loaded, setLoaded] = createSignal(false);
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const [saved, setSaved] = createSignal(false);
    return { stored, setStored, draft, setDraft, loaded, setLoaded, busy, setBusy, error, setError, saved, setSaved };
};
export type EnvDraft = ReturnType<typeof createEnvDraft>;

interface AgentEnvPanelProps {
    agentId: string;
    agentName: string;
    /** Given when hosted in a modal (My Agents' "Environment…"): shows a Close button. */
    onClose?: () => void;
    /** The list's copy of the box, so a rebuild of the panel keeps the edits. */
    draft?: EnvDraft;
}

export const AgentEnvPanel = (props: AgentEnvPanelProps): JSX.Element => {
    const { stored, setStored, draft, setDraft, loaded, setLoaded, busy, setBusy, error, setError, saved, setSaved } =
        props.draft ?? createEnvDraft();

    const dirty = () => draft() !== stored();
    const parsed = createMemo(() => parseEnvText(draft()));

    const load = async (): Promise<void> => {
        try {
            const content = await RpcApi.GetAgentContentCommand(TabRpcClient, {
                agent_id: props.agentId,
                content_type: ENV_CONTENT_TYPE,
            });
            const text = content?.content ?? "";
            const keepEdits = loaded() && dirty();
            setStored(text);
            if (!keepEdits) setDraft(text);
            setLoaded(true);
            setError(null);
        } catch (e) {
            setError(errorText(e));
        }
    };

    onMount(() => {
        void load();
        onCleanup(onMuxEvent(WpsEvent.AgentContentChanged, () => void load()));
    });

    const save = async (): Promise<void> => {
        setBusy(true);
        setError(null);
        setSaved(false);
        try {
            const text = draft();
            await RpcApi.SetAgentContentCommand(TabRpcClient, {
                agent_id: props.agentId,
                content_type: ENV_CONTENT_TYPE,
                content: text,
            });
            setStored(text);
            setSaved(true);
        } catch (e) {
            setError(errorText(e));
        } finally {
            setBusy(false);
        }
    };

    return (
        <div class="agent-primitive-modal-detail agent-env-panel" data-testid="agent-env-panel">
            <div class="agent-primitive-modal-readonly">
                <p class="agent-primitive-modal-global-note">{t("agentEnv.intro", { name: props.agentName })}</p>
                <EndpointNoticeLine agentId={props.agentId} />
                <Field label={t("agentEnv.label")} layout="stacked" hint={envHint()}>
                    <TextArea
                        mono
                        rows={8}
                        spellcheck={false}
                        placeholder={"EDITOR=vim\n# RUST_LOG=debug"}
                        value={draft()}
                        disabled={!loaded() || busy()}
                        onInput={(e) => {
                            setDraft(e.currentTarget.value);
                            setSaved(false);
                        }}
                    />
                </Field>
                <Show when={parsed().ignored.length > 0}>
                    <div class="agent-env-panel-notice" role="status" data-testid="agent-env-ignored">
                        <span class="agent-primitive-modal-field-label">{t("agentEnv.ignoredTitle")}</span>
                        <ul class="agent-env-panel-list">
                            <For each={parsed().ignored}>
                                {(line) => (
                                    <li>
                                        <Rich k="agentEnv.ignoredLine" params={{ number: line.number, text: line.text, reason: line.reason }} />
                                    </li>
                                )}
                            </For>
                        </ul>
                    </div>
                </Show>
                <Show when={parsed().reserved.length > 0}>
                    <p class="agent-primitive-modal-global-note" data-testid="agent-env-reserved">
                        {t("agentEnv.reserved", { names: parsed().reserved.join(", ") })}
                    </p>
                </Show>
                <Show when={error()}>
                    <div class="agent-primitive-modal-error" role="alert">
                        {error()}
                    </div>
                </Show>
                <Show when={!loaded() && error()}>
                    <div class="agent-primitive-modal-actions">
                        <Button onClick={() => void load()}>{t("agentEnv.retry")}</Button>
                    </div>
                </Show>
                <Show when={saved()}>
                    <p class="agent-primitive-modal-global-note" role="status">
                        {t("agentEnv.saved")}
                    </p>
                </Show>
                <div class="agent-primitive-modal-actions">
                    <Show when={props.onClose}>{(close) => <Button onClick={() => close()()}>{t("agentEnv.close")}</Button>}</Show>
                    <Button disabled={!dirty() || busy()} onClick={() => setDraft(stored())}>
                        {t("agentEnv.revert")}
                    </Button>
                    <Button tone="accent" busy={busy()} disabled={!dirty() || busy()} onClick={() => void save()}>
                        {t("agentEnv.save")}
                    </Button>
                </div>
            </div>
        </div>
    );
};

AgentEnvPanel.displayName = "AgentEnvPanel";
