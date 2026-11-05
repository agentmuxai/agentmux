// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * RestartToUpdatePopover: what the status bar's "Restart to update" chip opens
 * once srv has downloaded, verified and staged the newer release by itself
 * (`update/auto.rs`). What's new, Restart now, Later. Restart now is the same
 * request as the version panel's Upgrade & restart: srv waits for agents to
 * finish their turn, then the launcher asks in a dialog of its own, so a click
 * here is never the whole consent.
 * docs/specs/SPEC_BACKGROUND_UPDATES_ONE_CLICK_RESTART_2026_11_03.md §4.3.
 */

import { createSignal, Show, type JSX } from "solid-js";
import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { Button } from "@/app/element/ui";
import { cancelUpgrade, releaseNotesUrl, stagedRelease, upgradeAndRestart } from "@/store/app-update";
import { openLink } from "@/store/global";

interface RestartToUpdatePopoverProps {
    anchor: PopoverAnchor;
    onClose: () => void;
}

/** What the wait for agents to finish their turn says. */
export function waitingLine(n: number): string {
    return n === 1 ? "Waiting for 1 agent to finish its turn." : `Waiting for ${n} agents to finish their turn.`;
}

export const RestartToUpdatePopover = (props: RestartToUpdatePopoverProps): JSX.Element => {
    const staged = () => stagedRelease();
    const [error, setError] = createSignal<string | null>(null);
    const run = async (what: () => Promise<void>) => {
        setError(null);
        try {
            await what();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    const waiting = () => staged()?.state === "waiting";

    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-end"
            onDismiss={props.onClose}
            class="update-popover"
            role="dialog"
            aria-label="Restart to update AgentMux"
            style={{ width: "320px" }}
        >
            <div class="update-popover-title">AgentMux {staged()?.version} is ready</div>
            <div class="update-popover-note">
                It was downloaded and checked in the background. Restarting installs it; your agents resume where they were.
            </div>
            <Show when={waiting()}>
                <div class="update-popover-wait">{waitingLine(staged()?.waitingFor ?? 0)}</div>
            </Show>
            <Show when={error()}>
                <div class="update-popover-error">{error()}</div>
            </Show>
            <div class="update-popover-actions">
                <Button density="compact" onClick={() => void openLink(releaseNotesUrl(staged()?.version))}>
                    What's new
                </Button>
                <Show
                    when={waiting()}
                    fallback={
                        <>
                            <Button density="compact" onClick={props.onClose}>
                                Later
                            </Button>
                            <Button density="compact" tone="accent" onClick={() => void run(() => upgradeAndRestart(false))}>
                                Restart now
                            </Button>
                        </>
                    }
                >
                    <Button density="compact" onClick={() => void run(cancelUpgrade)}>
                        Cancel
                    </Button>
                    <Button density="compact" tone="accent" onClick={() => void run(() => upgradeAndRestart(true))}>
                        Restart anyway
                    </Button>
                </Show>
            </div>
        </AnchoredPopover>
    );
};
