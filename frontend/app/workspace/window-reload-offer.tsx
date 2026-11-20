// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The offer a window makes when its own display has gone wrong (a tab that stopped following the active one, panes the
 * layout holds that are not mounted, a burst of errors it could not contain): a line saying so and a way to reload it.
 * Never a reload of its own accord, which would lose a composer draft or a drag under way. The first two take
 * themselves back when the window recovers; the burst of errors stays until the person dismisses it.
 */

import { CloseButton } from "@/app/element/ui";
import { Button } from "@/app/element/ui/Button";
import { t } from "@/app/i18n/i18n";
import { getApi } from "@/store/global";
import { Show, type Accessor, type JSX } from "solid-js";

export type ReloadOfferReason = "tabs" | "panes" | "errors";

const MESSAGES: Record<ReloadOfferReason, () => string> = {
    tabs: () => t("window.reloadOffer.tabs"),
    panes: () => t("window.reloadOffer.panes"),
    errors: () => t("window.reloadOffer.errors"),
};

/** The offer to show of the ones standing: the tab that is wrong before panes missing before errors. */
export function chosenReloadOffer(standing: Record<ReloadOfferReason, boolean>): ReloadOfferReason | null {
    return standing.tabs ? "tabs" : standing.panes ? "panes" : standing.errors ? "errors" : null;
}

export function WindowReloadOffer(props: { reason: Accessor<ReloadOfferReason | null>; onDismiss: () => void }): JSX.Element {
    return (
        <Show when={props.reason()}>
            {(reason) => (
                <div
                    role="alert"
                    class="flex flex-row items-center gap-3"
                    style={{
                        position: "absolute",
                        bottom: "16px",
                        left: "50%",
                        "white-space": "nowrap",
                        transform: "translateX(-50%)",
                        "z-index": 20,
                        padding: "6px 10px",
                        "border-radius": "6px",
                        border: "1px solid var(--warning-color)",
                        background: "var(--modal-bg-color)",
                        "box-shadow": "0 4px 16px rgba(0, 0, 0, 0.4)",
                    }}
                >
                    <span>{MESSAGES[reason()]()}</span>
                    <Button tone="accent" density="compact" icon="rotate-right" onClick={() => getApi().reloadWindow()}>
                        {t("window.reloadOffer.reload")}
                    </Button>
                    <Show when={reason() === "errors"}>
                        <CloseButton label={t("window.reloadOffer.dismiss")} onClick={props.onDismiss} />
                    </Show>
                </div>
            )}
        </Show>
    );
}
