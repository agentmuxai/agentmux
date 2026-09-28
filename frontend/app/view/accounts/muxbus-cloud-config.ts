// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which Cognito settings the AgentMux Cloud sign-in uses: the build's
 * compiled ones, else the ones srv discovered from the relay
 * (`muxbus.cloudconfig`, SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.5).
 *
 * A build with a compiled client id (every production build) never asks srv,
 * so it behaves exactly as before. A build without one asks once and keeps a
 * client id once it has one; until then every `load()` asks again, which is
 * cheap because srv caches discovery for five minutes.
 */

import type { MuxBusCloudConfigResp } from "@/types/rpc/MuxBusCloudConfigResp";
import { createSignal, type Accessor } from "solid-js";

export interface MuxBusSignInSettings {
    cognitoDomain: string;
    clientId: string;
}

export interface MuxBusCloudConfig {
    /** The pair to sign in with, or null while neither the build nor srv has one. */
    signIn: Accessor<MuxBusSignInSettings | null>;
    /** Ask srv unless a pair is already known. Concurrent calls share one request; never rejects. */
    load: () => Promise<void>;
}

export function createMuxBusCloudConfig(
    compiled: MuxBusSignInSettings,
    fetchCloudConfig: () => Promise<MuxBusCloudConfigResp>
): MuxBusCloudConfig {
    const [discovered, setDiscovered] = createSignal<MuxBusSignInSettings | null>(null);
    let inflight: Promise<void> | null = null;

    const signIn = (): MuxBusSignInSettings | null => (compiled.clientId !== "" ? compiled : discovered());

    const load = (): Promise<void> => {
        if (signIn() !== null) return Promise.resolve();
        inflight ??= fetchCloudConfig()
            .then(
                (config) => {
                    if (config.clientId !== "" && config.cognitoDomain !== "") {
                        setDiscovered({ cognitoDomain: config.cognitoDomain, clientId: config.clientId });
                    }
                },
                () => {
                    // srv unreachable or an older srv without the command:
                    // not configured, and the next load() asks again.
                }
            )
            .finally(() => {
                inflight = null;
            });
        return inflight;
    };

    return { signIn, load };
}
