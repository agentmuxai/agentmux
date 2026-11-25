// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A jekt that waits on the person, as a card in the agent's pane, docked above the composer
 * (docs/specs/SPEC_JEKT_SIDE_PANEL_RESEARCH_AND_DESIGN_2026_11_04.md §5.4): a `TIER=sensitive` jekt with
 * `ESCALATE=required`, which the agent must not act on until the person confirms in their own conversation. The
 * card names the sender as the marker does, shows how far it was proven, and takes the person to the message. It
 * ends when the person next sends in the pane or dismisses it; the tone, the badge and the notification come from
 * the wait it registers.
 */

import { Show, type JSX } from "solid-js";

import type { JektMessageNode } from "@/app/agent-model/node-types";
import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { PaneRequestProps } from "@/app/store/pane-requests";
import { trustLine, trustState } from "./jekt-trust-wording";
import "./JektNeedsYouCard.scss";

export interface JektNeedsYouProps {
    /** The newest waiting jekt. */
    jekt: JektMessageNode;
    /** How many more wait, older than it. */
    earlier: number;
    /** Scroll the transcript to a jekt. */
    onShow: (nodeId: string) => void;
    /** The person has seen them: end the wait for every one waiting now. */
    onDismiss: () => void;
}

export function JektNeedsYouCard(props: JektNeedsYouProps & PaneRequestProps): JSX.Element {
    const sender = () => props.jekt.from;
    return (
        <section class="jekt-needs-you-card" aria-label={t("jektNeedsYou.label", { sender: sender() })}>
            <h3 class="jekt-needs-you-title">{t("jektNeedsYou.title", { sender: sender() })}</h3>
            <div class="jekt-needs-you-body">
                <p class="jekt-needs-you-trust" data-trust={trustState(props.jekt.trust, props.jekt.sig)}>
                    {trustLine(props.jekt.trust, props.jekt.sig)}
                </p>
                <Show when={props.earlier > 0}>
                    <p class="jekt-needs-you-more">{t("jektNeedsYou.earlier", { count: props.earlier })}</p>
                </Show>
                <p>
                    {t("jektNeedsYou.body")}
                </p>
            </div>
            <div class="jekt-needs-you-actions">
                <Button
                    onClick={() => {
                        props.onDismiss();
                        props.close();
                    }}
                >
                    {t("jektNeedsYou.dismiss")}
                </Button>
                <Button tone="attention" onClick={() => props.onShow(props.jekt.id)}>
                    {t("jektNeedsYou.show")}
                </Button>
            </div>
        </section>
    );
}
