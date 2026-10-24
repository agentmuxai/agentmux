// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DesktopDrivingBar: while an agent may control a native app, its pane says so
 * and offers Stop (docs/specs/SPEC_DESKTOP_DRIVING_2026_10_20.md §5.6).
 *
 * srv publishes the grants in force (`app:desktop`); this shows the ones for
 * this pane. Stop goes to srv the one way it can, through the host
 * (`approvals.stopDesktop`): the agent's grants end and its next action is
 * told so.
 */

import { createSignal, For, Show, type JSX } from "solid-js";

import { desktopGrants } from "@/app/block/desktop-requests";
import { getApi } from "@/app/store/app-api";
import { Button } from "@/app/element/ui";

export const DesktopDrivingBar = (props: { blockId: string }): JSX.Element => {
    const mine = () => desktopGrants().filter((g) => g.block_id === props.blockId);
    const [error, setError] = createSignal<string | null>(null);
    const stop = async () => {
        try {
            await getApi().approvals.stopDesktop!(props.blockId);
            setError(null);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <Show when={mine().length > 0}>
            <div class="agent-desktop-bar" role="status" aria-live="polite">
                <span class="agent-desktop-bar-message">
                    <For each={mine()}>
                        {(g, i) => (
                            <span class="agent-desktop-bar-app" classList={{ "is-active": g.active }}>
                                {i() > 0 ? ", " : ""}
                                {g.active ? "Driving " : "May drive "}
                                <strong>{g.app}</strong>
                            </span>
                        )}
                    </For>
                </span>
                {error() && <span class="agent-desktop-bar-error">{error()}</span>}
                <Button tone="danger" onClick={() => void stop()}>
                    Stop
                </Button>
            </div>
        </Show>
    );
};
