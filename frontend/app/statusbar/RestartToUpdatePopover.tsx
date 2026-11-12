// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * RestartToUpdatePopover: what the status bar's "Restart to update" chip opens
 * once srv has downloaded, verified and staged the newer release by itself
 * (`update/auto.rs`). What's new, Restart now, Later. Restart now is the same
 * request as the version panel's Upgrade & restart: srv waits for agents to
 * finish their turn, then the launcher asks in a dialog of its own, so a click
 * here is never the whole consent.
 *
 * While srv waits, the panel names who it waits for and offers Cancel and
 * Restart anyway; once the wait ends the restart starts by itself and the
 * panel says "Restarting now". A wait for nobody is never shown: it is not a
 * wait.
 * docs/specs/SPEC_BACKGROUND_UPDATES_ONE_CLICK_RESTART_2026_11_03.md §4.3.
 */

import { createEffect, createSignal, For, Show, type JSX } from "solid-js";
import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { Button } from "@/app/element/ui";
import { cancelUpgrade, releaseDownload, releaseNotesUrl, stagedRelease, upgradeAndRestart } from "@/store/app-update";
import { openLink } from "@/store/global";
import { updateWait } from "./update-wait";

interface RestartToUpdatePopoverProps {
    anchor: PopoverAnchor;
    onClose: () => void;
}

/** What the panel says once the launcher has been asked: only its dialog decides from here. */
export const RESTARTING_LINE = "Restarting now: answer the AgentMux dialog (No cancels).";

let saidWaitsForNobody = false;

/** srv said it waits, and named nobody: said once, with what srv sent, so the cause can be found. */
function noteWaitForNobody(): void {
    if (saidWaitsForNobody) return;
    saidWaitsForNobody = true;
    console.warn("[update] the restart is waiting, but for nobody; showing Restart now.", JSON.stringify(releaseDownload()));
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
    /** Who the restart waits for; null when it isn't waiting, or waits for nobody. */
    const wait = () => {
        const s = staged();
        return s?.state === "waiting" ? updateWait(s.waitingOn, s.waitingFor) : null;
    };
    const restarting = () => staged()?.state === "restarting";
    createEffect(() => {
        if (staged()?.state === "waiting" && !wait()) noteWaitForNobody();
    });

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
            <Show when={wait()}>
                <For each={wait()!.lines}>{(line) => <div class="update-popover-wait">{line}</div>}</For>
                <div class="update-popover-note">The restart starts by itself then. {wait()!.interrupts}</div>
            </Show>
            <Show when={restarting()}>
                <div class="update-popover-wait">{RESTARTING_LINE}</div>
            </Show>
            <Show when={error()}>
                <div class="update-popover-error">{error()}</div>
            </Show>
            <div class="update-popover-actions">
                <Button density="compact" onClick={() => void openLink(releaseNotesUrl(staged()?.version))}>
                    What's new
                </Button>
                <Show when={!restarting()}>
                    <Show
                        when={wait()}
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
                </Show>
            </div>
        </AnchoredPopover>
    );
};
