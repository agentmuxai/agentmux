// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Check now" on an agent marked out of quota: when the plan was raised or the quota refilled before the
 * provider's reset, nothing tells the agent, and it stays out of quota until that reset. This asks srv to
 * look again now: a small turn on the agent, and a turn that works clears the limit (and wakes what was
 * held for it). The agent API for the same thing is `Identity` with action `check_limits`.
 */

import { createSignal, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Button } from "@/app/element/ui";
import { tip } from "@/app/element/tip";

/** How long the button says "Checking…" before it is offered again if the limit is still shown. */
export const CHECKING_MS = 30_000;

/** Starts the check on `blockIds`; the first reason one could not be started, or `null`. */
export async function checkLimits(blockIds: string[]): Promise<string | null> {
    try {
        const result = await RpcApi.FleetCheckLimitsCommand(TabRpcClient, { targets: blockIds });
        return result.failed[0]?.error ?? null;
    } catch (e) {
        return e instanceof Error ? e.message : String(e);
    }
}

/** The words of the button for its state. */
export function checkNowText(checking: boolean, error: string | null): string {
    if (checking) return "Checking…";
    return error ? "Check failed" : "Check now";
}

/** The button beside "Out of quota": looks at the agent again now instead of waiting for the reset. */
export function CheckNowButton(props: { blockId: string; check?: (blockIds: string[]) => Promise<string | null> }): JSX.Element {
    const [checking, setChecking] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const run = props.check ?? checkLimits;
    const click = async (e: MouseEvent) => {
        e.stopPropagation();
        if (checking()) return;
        setChecking(true);
        const failed = await run([props.blockId]);
        setError(failed);
        if (failed) {
            setChecking(false);
            return;
        }
        setTimeout(() => setChecking(false), CHECKING_MS);
    };
    return (
        <span
            class="ui-tooltip-anchor"
            {...tip(
                error() ??
                    "Looks at this agent again now, in case your quota is back before the reset. It starts a small turn; if it works, the agent can take messages again."
            )}
        >
            <Button
                class="swarm-unavailable-check"
                tone="attention"
                density="compact"
                icon="rotate"
                busy={checking()}
                aria-label={checkNowText(checking(), error())}
                onClick={click}
            >
                <span class="swarm-unavailable-check-text">{checkNowText(checking(), error())}</span>
            </Button>
        </span>
    );
}
