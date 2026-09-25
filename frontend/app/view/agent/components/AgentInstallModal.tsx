// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentInstallModalPanel — modal that runs an agent's install recipe.
 * Opens when the user picks an agent whose CLI isn't already in the
 * per-version cache. Sibling to `AgentLaunchModalPanel`.
 *
 * Phase α (SPEC_AGENT_INSTALL_STAGE_2026_05_17.md §11): single-step
 * recipe (just `npm install <package>`) streamed line-by-line via the
 * `install.start` RPC. Cancel kills the install + removes the partial
 * dir.
 *
 * Rendered with the shared `<InstallDialog>` (SPEC_UNIVERSAL_INSTALL_
 * DIALOG_2026_09_23.md §5): plain-language npm steps by default, the raw
 * console under a collapsed "Details". This file only owns what is
 * specific to provider CLIs — which provider to install, and what the
 * footer buttons do.
 */

import { Match, Switch, createResource, onCleanup, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { InstallDialog } from "@/element/install/InstallDialog";
import { createInstallSession } from "@/element/install/install-session";
import { NpmStepTracker } from "@/element/install/npm-steps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { getProvider } from "../providers";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import type { AgentDefinition } from "@/app/store/rpc-api";

interface AgentInstallModalPanelProps {
    agent: AgentDefinition;
    onCancel: () => void;
    /**
     * Fires when the install completed successfully. The boolean tells
     * the caller whether the user clicked "Continue to Launch" (true)
     * or "Close" (false). The picker uses the false case to still flip
     * its cached install state so the ribbon goes away even when the
     * user dismisses the success screen — codex caught this on PR #895.
     */
    onInstalled: (continueToLaunch: boolean) => void;
}

// Header subtitle per state: what the install needs before it runs,
// then where it stands.
const DESCRIPTION = {
    idle: "Needs an internet connection.",
    running: "Needs an internet connection.",
    done: "Ready to launch.",
    failed: "The install didn't finish.",
} as const;

export const AgentInstallModalPanel = (props: AgentInstallModalPanelProps): JSX.Element => {
    // Resolve through the agent's bound bundle rather than the possibly-
    // drifted `agent.provider` column directly — #2594, same "gate vs.
    // actual launch can disagree" risk class #2592/#2596/#2607/#2609
    // fixed. This modal determines which CLI package literally gets
    // installed; disagreeing with what AgentPicker's checkInstalled
    // (already fixed) decided needed installing would install the wrong
    // provider's CLI.
    //
    // Used only for the cosmetic header (icon/displayName/version) —
    // `start` re-resolves directly rather than reading this resource, so a
    // click that races the resource's own in-flight fetch still installs
    // the correct provider. Falls back to `props.agent.provider` while
    // loading/on failure, same as `resolveEffectiveLaunchProvider` itself.
    const [resolvedProviderId] = createResource(() => props.agent, resolveEffectiveLaunchProvider);
    const displayProviderId = () => resolvedProviderId() ?? props.agent.provider;
    const catalog = () => getCliCatalogEntry(displayProviderId());
    const provider = () => getProvider(displayProviderId());
    const displayName = () => catalog()?.displayName ?? props.agent.name;
    const version = () => {
        const v = provider()?.pinnedVersion;
        if (!v) return undefined;
        return v === "latest" ? "latest" : `v${v}`;
    };

    // The provider this run installs, resolved fresh on each click.
    let runProviderId: string | null = null;

    const session = createInstallSession({
        tracker: () =>
            new NpmStepTracker(
                (runProviderId && getCliCatalogEntry(runProviderId)?.displayName) || displayName(),
            ),
        begin: async () => {
            const prov = runProviderId ? getProvider(runProviderId) : undefined;
            if (!prov) throw new Error(`unknown provider ${runProviderId}`);
            return RpcApi.InstallStartCommand(TabRpcClient, {
                providerId: prov.id,
                cliCommand: prov.cliCommand,
                npmPackage: prov.npmPackage,
                pinnedVersion: prov.pinnedVersion,
            });
        },
        cancel: (sessionId) => RpcApi.InstallCancelCommand(TabRpcClient, { sessionId }),
        // npm into an isolated per-version directory is safe to stop; the
        // backend rolls back the partial directory.
        cancelOnDispose: true,
    });

    const start = async () => {
        // Re-resolve directly rather than reading the `provider()` memo
        // above — that memo backs the resource's current (possibly still-
        // loading, or subsequently-stale if the component has been open a
        // while) snapshot, whereas a fresh resolve here guarantees whatever
        // actually gets installed matches the agent's bundle at the moment
        // the user clicked, not whatever the header happened to be showing.
        runProviderId = await resolveEffectiveLaunchProvider(props.agent);
        await session.start();
    };

    // Set when either footer button fires onInstalled, so the unmount
    // path in onCleanup doesn't double-fire. Also lets us detect "user
    // dismissed the success screen via ESC / backdrop" (notifiedDone
    // stays false in those paths) and flip state once on the way out.
    // Codex P2 on PR #895.
    let notifiedDone = false;
    onCleanup(() => {
        if (session.state() === "done" && !notifiedDone) props.onInstalled(false);
    });
    const installed = (continueToLaunch: boolean) => {
        notifiedDone = true;
        props.onInstalled(continueToLaunch);
    };

    return (
        <InstallDialog
            session={session}
            kind="provider-cli"
            icon={catalog()?.icon ?? "📦"}
            title={session.state() === "done" ? `${displayName()} is installed` : `Install ${displayName()}`}
            version={version()}
            description={DESCRIPTION[session.state()]}
            actions={
                <Switch>
                    <Match when={session.state() === "idle"}>
                        <Button onClick={() => props.onCancel()} data-modal-dismiss>Cancel</Button>
                        <Button onClick={() => void start()} className="green solid" data-modal-initial-focus>
                            Install now
                        </Button>
                    </Match>
                    <Match when={session.state() === "running"}>
                        <Button
                            onClick={async () => {
                                await session.cancel();
                                props.onCancel();
                            }}
                            data-modal-dismiss
                        >
                            Cancel
                        </Button>
                    </Match>
                    <Match when={session.state() === "failed"}>
                        <Button onClick={() => props.onCancel()} data-modal-dismiss>Close</Button>
                        <Button onClick={() => void start()} className="green solid">
                            Retry
                        </Button>
                    </Match>
                    <Match when={session.state() === "done"}>
                        <Button onClick={() => installed(false)} data-modal-dismiss>Close</Button>
                        <Button onClick={() => installed(true)} className="green solid">
                            Continue to Launch
                        </Button>
                    </Match>
                </Switch>
            }
        />
    );
};

AgentInstallModalPanel.displayName = "AgentInstallModalPanel";
