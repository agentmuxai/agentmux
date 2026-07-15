// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCredentialsRevokedChip — layer-2 disclosure chip for the agent pane
 * (SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §3).
 *
 * Subscribes to the per-agent `agentcredentials:revoked:<agentId>` event the
 * backend publishes when an identity account this agent was linked to is
 * deleted (or the link is removed) while the agent may still be running.
 * Deleting the account does NOT deauthenticate a live CLI process — it
 * already read its tokens — so this chip *discloses* that honestly rather
 * than pretending the revocation took effect. Enforcement lands at the next
 * spawn (layer 3, parallel PR).
 *
 * Persistent (survives until dismissed or the pane unmounts; deliberately
 * NOT persisted across app restarts — a restart is exactly what clears the
 * condition) and dismissable via the × action. Rendered through the shared
 * PaneRow pin primitive so it matches the other agent-pane status rows
 * (failure-recovery, runtime pin).
 */

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { PaneRow } from "./PaneRow";

interface AgentCredentialsRevokedChipProps {
    /** AgentDefinition id — the scope of the revocation event. */
    agentId: string;
}

interface RevokedState {
    /** Providers whose credentials were revoked while this pane was open
     *  (accumulated across events; usually a single entry). */
    providers: string[];
}

export const AgentCredentialsRevokedChip = (
    props: AgentCredentialsRevokedChipProps,
): JSX.Element => {
    const [revoked, setRevoked] = createSignal<RevokedState | null>(null);

    // agentId is stable for the lifetime of AgentPresentationView (the
    // wrapper remounts on change), so a plain subscribe/cleanup pair is
    // enough — mirrors AgentIdentityLinksPanel's subscription to the
    // sibling `agentidentities:changed:<id>` event.
    const unsub = waveEventSubscribe({
        eventType: `agentcredentials:revoked:${props.agentId}`,
        handler: (event: WaveEvent) => {
            const provider =
                typeof event?.data?.provider === "string" ? event.data.provider : "";
            setRevoked((prev) => {
                const providers = new Set(prev?.providers ?? []);
                if (provider) providers.add(provider);
                return { providers: [...providers] };
            });
        },
    });
    onCleanup(unsub);

    return (
        <Show when={revoked()}>
            {(state) => (
                <PaneRow
                    sigil="⚠"
                    accent="error"
                    title="Credentials revoked — this agent still holds tokens until restarted."
                    meta={state().providers.join(", ") || undefined}
                    actions={[
                        {
                            glyph: "×",
                            title: "Dismiss",
                            onClick: () => setRevoked(null),
                        },
                    ]}
                />
            )}
        </Show>
    );
};

AgentCredentialsRevokedChip.displayName = "AgentCredentialsRevokedChip";
