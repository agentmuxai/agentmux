// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentContainerCaches: the "Build cache" section of the Container limits panel. A container agent on the developer
 * image keeps its cargo registry and build output in volumes that outlive the container; this shows how much they hold
 * and clears them (docs/specs/SPEC_CONTAINER_AGENT_DEVELOPER_IMAGE_2026_10_29.md section 5). Nothing shows for an agent
 * that keeps no cache, and nothing when Docker does not answer.
 */

import { createSignal, onMount, Show, type JSX } from "solid-js";

import { Button, Field } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentContainerCachesResult } from "@/types/rpc/AgentContainerCachesResult";
import { formatBytes } from "@/util/format-bytes";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

interface AgentContainerCachesProps {
    agentId: string;
    agentName: string;
}

export const AgentContainerCaches = (props: AgentContainerCachesProps): JSX.Element => {
    const [caches, setCaches] = createSignal<AgentContainerCachesResult | null>(null);
    const [confirming, setConfirming] = createSignal(false);
    const [busy, setBusy] = createSignal(false);
    const [freed, setFreed] = createSignal<number | null>(null);
    const [error, setError] = createSignal<string | null>(null);

    onMount(() => {
        void (async () => {
            try {
                setCaches(await RpcApi.GetContainerCachesCommand(TabRpcClient, { agent_id: props.agentId }));
            } catch {
                // Docker not answering is not this section's news (the limits above say it): the section stays hidden.
            }
        })();
    });

    const clear = async (): Promise<void> => {
        setBusy(true);
        setError(null);
        try {
            const result = await RpcApi.ClearContainerCachesCommand(TabRpcClient, { agent_id: props.agentId });
            setCaches(result);
            setFreed(result.freed_bytes ?? 0);
            setConfirming(false);
        } catch (e) {
            setError(errorText(e));
        } finally {
            setBusy(false);
        }
    };

    const total = () => caches()?.total_bytes ?? 0;
    const count = () => caches()?.volumes.length ?? 0;
    const large = () => caches() != null && total() >= (caches()?.warn_bytes ?? Infinity);

    return (
        <Show when={count() > 0 || freed() != null || error()}>
            <Field label={t("containerCaches.label")} layout="stacked">
                <div data-testid="container-caches">
                    <Show when={count() > 0}>
                        <p class="agent-primitive-modal-global-note" data-testid="container-caches-size">
                            {t("containerCaches.size", { size: formatBytes(total()), count: count(), name: props.agentName })}
                        </p>
                        <Show when={large()}>
                            <p class="agent-primitive-modal-global-note" role="status" data-testid="container-caches-large">
                                {t("containerCaches.large")}
                            </p>
                        </Show>
                        <div class="agent-primitive-modal-actions">
                            <Show
                                when={confirming()}
                                fallback={
                                    <Button disabled={busy()} onClick={() => setConfirming(true)} data-testid="container-caches-clear">
                                        {t("containerCaches.clear")}
                                    </Button>
                                }
                            >
                                <Button tone="danger" busy={busy()} onClick={() => void clear()} data-testid="container-caches-confirm">
                                    {t("containerCaches.remove", { size: formatBytes(total()) })}
                                </Button>
                                <Button disabled={busy()} onClick={() => setConfirming(false)}>
                                    {t("containerCaches.cancel")}
                                </Button>
                            </Show>
                        </div>
                    </Show>
                    <Show when={freed() != null}>
                        <p class="agent-primitive-modal-global-note" role="status" data-testid="container-caches-cleared">
                            {t("containerCaches.cleared", { size: formatBytes(freed() ?? 0) })}
                        </p>
                    </Show>
                    <Show when={error()}>
                        <div class="agent-primitive-modal-error" role="alert" data-testid="container-caches-error">
                            {error()}
                        </div>
                    </Show>
                </div>
            </Field>
        </Show>
    );
};

AgentContainerCaches.displayName = "AgentContainerCaches";
