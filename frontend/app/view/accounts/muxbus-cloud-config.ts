// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which Cognito settings the AgentMux Cloud sign-in uses: the build's
 * compiled ones, else the ones srv discovered from the relay
 * (`muxbus.cloudconfig`, SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.5),
 * and which social sign-ins (Google, Apple) the cloud publishes.
 *
 * A build with a compiled client id (every production build) signs in with it
 * whatever srv says, but still asks srv for the published sign-ins. Every
 * `load()` asks again until srv has a document, which is cheap because srv
 * caches discovery for five minutes; after that it never asks again.
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
    /** The social sign-ins the cloud publishes (`Google`, `SignInWithApple`); empty until srv has a document. */
    identityProviders: Accessor<string[]>;
    /** Ask srv unless its document is already known. Concurrent calls share one request; never rejects. */
    load: () => Promise<void>;
}

export function createMuxBusCloudConfig(
    compiled: MuxBusSignInSettings,
    fetchCloudConfig: () => Promise<MuxBusCloudConfigResp>
): MuxBusCloudConfig {
    const [discovered, setDiscovered] = createSignal<MuxBusSignInSettings | null>(null);
    const [identityProviders, setIdentityProviders] = createSignal<string[]>([]);
    let documentKnown = false;
    let inflight: Promise<void> | null = null;

    const signIn = (): MuxBusSignInSettings | null => (compiled.clientId !== "" ? compiled : discovered());

    const load = (): Promise<void> => {
        if (documentKnown && signIn() !== null) return Promise.resolve();
        inflight ??= fetchCloudConfig()
            .then((config) => {
                if (config.clientId !== "" && config.cognitoDomain !== "") {
                    setDiscovered({ cognitoDomain: config.cognitoDomain, clientId: config.clientId });
                }
                if (config.source === "discovered") {
                    // An older srv doesn't send the list.
                    setIdentityProviders(config.identityProviders ?? []);
                    documentKnown = true;
                }
            })
            .catch(() => {
                // srv unreachable or an older srv without the command:
                // nothing learned, and the next load() asks again.
            })
            .finally(() => {
                inflight = null;
            });
        return inflight;
    };

    return { signIn, identityProviders, load };
}
