// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent's request to install a widget, as a card in its own pane, docked above the composer
 * (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 5, D4). Drawn from srv's own record of the
 * request (the widget's manifest and signer), never from the agent's words, and operator-only: the dock
 * marks it so an agent's UI query, click and keys never reach it. The tone, badges and notification come
 * from srv's attention event for the same pane, so the card adds no wait of its own.
 */

import { createSignal, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import type { PaneRequestProps } from "@/app/store/pane-requests";
import type { WidgetInstallRequest } from "@/app/store/rpc-api/widgets";
import { installLabel, WidgetApprovalDetails } from "@/app/view/settings/sections/widget-approval-details";
import "./widget-install-card.scss";

export function WidgetInstallCard(props: { request: WidgetInstallRequest; decide: (approve: boolean) => Promise<void> } & PaneRequestProps): JSX.Element {
    const r = props.request;
    const [error, setError] = createSignal<string | null>(null);
    const answer = async (approve: boolean) => {
        try {
            await props.decide(approve);
            props.close();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <section class="widget-install-card" aria-label={`${r.agent} wants to install ${r.name}`}>
            <h3 class="widget-install-card-title">
                {r.agent} wants to install {r.name}
            </h3>
            <div class="widget-install-card-body">
                <WidgetApprovalDetails pkg={r} />
                {error() && (
                    <div class="settings-config-error" role="alert">
                        {error()}
                    </div>
                )}
            </div>
            <div class="widget-install-card-actions">
                <Button onClick={() => void answer(false)}>Don't install</Button>
                {/* Not signed by the publisher's usual key: the risky choice isn't the bright one. */}
                <Button tone={r.signature?.state === "key_changed" ? "danger" : "attention"} onClick={() => void answer(true)}>
                    {installLabel(r)}
                </Button>
            </div>
        </section>
    );
}
