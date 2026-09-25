// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Covers a closing agent pane with its shutdown log (§5.5): what srv stopped,
 * line by line, until the pane leaves the layout. A failed close keeps the
 * pane and offers to try again.
 */

import { For, onCleanup, Show } from "solid-js";
import * as services from "@/store/services";
import { beginShutdownLog, endShutdownLog, shutdownLogFor, visibleShutdownLines } from "./shutdown-log";
import "./ShutdownOverlay.scss";

export function ShutdownOverlay(props: { blockId: string; agentName?: string }) {
    // The pane unmounts when srv's `delete` action removes it: stop listening.
    onCleanup(() => endShutdownLog(props.blockId));
    const log = () => shutdownLogFor(props.blockId);
    const view = () => visibleShutdownLines(log()?.lines ?? []);

    const retry = () => {
        const id = props.blockId;
        endShutdownLog(id);
        beginShutdownLog(id);
        services.ObjectService.ClosePane([id], true).catch(() => {});
    };

    return (
        <Show when={log()}>
            {(l) => (
                <div class="agent-shutdown-overlay" role="status" aria-live="polite">
                    <div class="agent-shutdown-overlay__panel">
                        <div class="agent-shutdown-overlay__title">
                            {l().error ? "Couldn't shut down" : `Shutting down ${props.agentName || "agent"}…`}
                        </div>
                        <Show when={view().more > 0}>
                            <div class="agent-shutdown-overlay__line agent-shutdown-overlay__more">
                                +{view().more} more
                            </div>
                        </Show>
                        <For each={view().shown}>
                            {(line) => (
                                <div
                                    class="agent-shutdown-overlay__line"
                                    classList={{ "agent-shutdown-overlay__line--error": line.step === "error" }}
                                >
                                    {line.text}
                                </div>
                            )}
                        </For>
                        <Show when={l().error}>
                            <div class="agent-shutdown-overlay__actions">
                                <button type="button" class="modal-btn" onClick={() => endShutdownLog(props.blockId)}>
                                    Keep open
                                </button>
                                <button type="button" class="modal-btn modal-btn--confirm modal-btn--destructive" onClick={retry}>
                                    Try again
                                </button>
                            </div>
                        </Show>
                    </div>
                </div>
            )}
        </Show>
    );
}
