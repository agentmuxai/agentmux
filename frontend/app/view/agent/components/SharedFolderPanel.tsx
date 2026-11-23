// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The panel in the picker that is opening a host agent in a folder a container agent can write: names both, says what
 * the shared folder means, and lets the person go ahead (kept for that pair of agents) or not open it
 * (`shared-folder-ask.ts`).
 */

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { Overlap } from "@/types/rpc/Overlap";
import { onMount, Show, type JSX } from "solid-js";
import { keepInViewWhileItSettles } from "./create-panel-install-view";

export interface SharedFolderPanelProps {
    hostName: string;
    overlaps: Overlap[];
    busy: boolean;
    onGoAhead: () => void;
    onKeepApart: () => void;
}

export function SharedFolderPanel(props: SharedFolderPanelProps): JSX.Element {
    const containers = () => [...new Set(props.overlaps.map((pair) => pair.container_name))].join(", ");
    let root: HTMLDivElement | undefined;
    // It opens below the agent cards, where the person's click on Open did not look: bring it into view, and keep it
    // there while the cards above it finish loading.
    onMount(() => keepInViewWhileItSettles(root));
    return (
        <div ref={root} class="agent-shared-folder-panel" role="group" aria-label={t("agentPicker.sharedFolder.title")} data-testid="shared-folder-panel">
            <div class="shared-folder-icon">
                <i class="fa-solid fa-folder-open" />
            </div>
            <div class="shared-folder-content">
                <div class="shared-folder-title">{t("agentPicker.sharedFolder.title")}</div>
                <div class="shared-folder-text" data-testid="shared-folder-text">
                    {t("agentPicker.sharedFolder.body", { containers: containers(), host: props.hostName })}
                </div>
                <div class="shared-folder-hint">{t("agentPicker.sharedFolder.hint")}</div>
                <Show when={props.overlaps.some((pair) => pair.untrusted)}>
                    <div class="shared-folder-hint" data-testid="shared-folder-untrusted">
                        {t("agentPicker.sharedFolder.untrusted")}
                    </div>
                </Show>
                <div class="shared-folder-actions">
                    <Button tone="neutral" onClick={() => props.onKeepApart()} disabled={props.busy} data-testid="shared-folder-cancel">
                        {t("agentPicker.sharedFolder.cancel")}
                    </Button>
                    <Button tone="accent" busy={props.busy} onClick={() => props.onGoAhead()} data-testid="shared-folder-go-ahead">
                        {t("agentPicker.sharedFolder.goAhead")}
                    </Button>
                </div>
            </div>
        </div>
    );
}
