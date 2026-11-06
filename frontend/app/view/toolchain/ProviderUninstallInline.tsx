// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ProviderUninstallInline: Uninstall for an agent CLI whose running copy is the
 * one AgentMux installed. It removes that copy and nothing else: the person's
 * own copy on PATH, the CLI's sign-in and its other versions stay
 * (docs/specs/SPEC_ISOLATED_INSTALL_FOR_NON_NPM_CLIS_2026_11_05.md §4). A copy
 * a running agent uses is refused by srv, and its words are shown here.
 */

import { createSignal, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { invalidateInstallCheck } from "@/app/view/agent/install-check-cache";

interface ProviderUninstallInlineProps {
    providerId: string;
    label: string;
    /** AgentMux's copy is gone: the row looks again. */
    onRemoved: () => void;
}

export const ProviderUninstallInline = (props: ProviderUninstallInlineProps): JSX.Element => {
    const [asking, setAsking] = createSignal(false);
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string>();
    const remove = async () => {
        setBusy(true);
        setError(undefined);
        try {
            const result = await RpcApi.InstallUninstallCommand(TabRpcClient, { providerId: props.providerId });
            if (!result.removed) {
                // The copy in use is an older AgentMux version's own (instances/v<version>/cli), not the shared one.
                setError(`Nothing was removed: this copy of ${props.label} is in an older AgentMux version's folder, which Uninstall does not touch.`);
                return;
            }
            invalidateInstallCheck(props.providerId);
            props.onRemoved();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
            setAsking(false);
        }
    };
    return (
        <>
            {/* The question is its own line, so Remove and Cancel stay together under it. */}
            <Show when={asking()}>
                <p class="toolchain-detail-note">
                    Remove AgentMux's copy of {props.label}? Your own copy and your sign-in are not touched.
                </p>
            </Show>
            <div class="toolchain-detail-row">
                <Show
                    when={asking()}
                    fallback={
                        <Button onClick={() => setAsking(true)} disabled={busy()}>
                            Uninstall
                        </Button>
                    }
                >
                    <Button tone="accent" onClick={() => void remove()} disabled={busy()}>
                        Remove
                    </Button>
                    <Button onClick={() => setAsking(false)} disabled={busy()}>
                        Cancel
                    </Button>
                </Show>
            </div>
            <Show when={error()}>{(msg) => <p class="toolchain-detail-note">{msg()}</p>}</Show>
        </>
    );
};

ProviderUninstallInline.displayName = "ProviderUninstallInline";
