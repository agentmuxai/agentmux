// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ProviderUpdateInline: the "Update to the version AgentMux validated" step for
 * an agent CLI that AgentMux installs and that is behind its pin. It runs the
 * same install as the new-agent panel's (`agentInstallSession`, which installs
 * the registry's pinned version), so the run outlives this card, and when it
 * ends the install check is forgotten and the row looks again.
 */

import { Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { InstallProgress } from "@/element/install/InstallProgress";
import { agentInstallSession } from "@/app/view/agent/install-sessions";
import "./SystemToolInstallInline.scss";

interface ProviderUpdateInlineProps {
    providerId: string;
    /** The version AgentMux validated: what the update installs. */
    pinned: string;
}

export const ProviderUpdateInline = (props: ProviderUpdateInlineProps): JSX.Element => {
    const session = () => agentInstallSession(props.providerId);
    return (
        <div class="system-tool-install-inline">
            <Show
                when={session().state() !== "idle"}
                fallback={
                    <Button tone="accent" onClick={() => void session().start()}>
                        {`Update to ${props.pinned}`}
                    </Button>
                }
            >
                <div class="system-tool-install-status">
                    {session().state() === "running"
                        ? "Updating…"
                        : session().state() === "done"
                          ? "Updated"
                          : "Failed"}
                </div>
                <InstallProgress session={session()} kind="provider-cli" />
                <Show when={session().state() === "failed"}>
                    <div>
                        <Button onClick={() => void session().start()}>Retry</Button>
                    </div>
                </Show>
            </Show>
        </div>
    );
};

ProviderUpdateInline.displayName = "ProviderUpdateInline";
