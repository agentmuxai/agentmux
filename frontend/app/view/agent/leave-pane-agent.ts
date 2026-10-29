// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Taking an agent out of a pane in place (Back to My Agents, Start a new session): what the pane keeps of it, and
 * what happens to its CLI.
 *
 * Both used to be wrong. The CLI was left running, so a container agent went on being the agent of a pane that had
 * been handed to someone else, and the pane kept everything the old agent's launch had written that the picker
 * does not clear: its mode, its container image, its command and environment, its session id. The next agent
 * launched into the pane merged its own keys over those. A host agent launched into a container agent's pane kept
 * the container image (so the pane showed a container's update banner), and was started natively with the previous
 * agent's credentials directory in its environment (#10823).
 *
 * `close-agent-tab.ts` already refused this for closing the last tab, because it "leaves the CLI running and stale
 * meta behind"; Back and Start a new session had no such guard.
 */

/** Meta keys that describe the agent in the pane, or its launch. A null deletes the key (srv `merge_meta`). */
export const LEFT_AGENT_META: Record<string, null> = {
    // who it is
    agentId: null,
    agentProvider: null,
    agentOutputFormat: null,
    agentName: null,
    agentIcon: null,
    agentCliPath: null,
    agentCliArgs: null,
    agentBinDir: null,
    controller: null,
    // Identity M4b-3 (spec §6.5.8): a stamp left behind would name the previous launch's row to the next one.
    agentInstanceId: null,
    // how it was launched: a host or container agent, and the container's details
    agentMode: null,
    "agent:container_image": null,
    "agent:container_volumes": null,
    "agent:container_command": null,
    // what was started, and with what
    cmd: null,
    "cmd:args": null,
    "cmd:cwd": null,
    "cmd:env": null,
    "agent:runtime": null,
    "agent:provider_flags": null,
    "agent:resume_flag": null,
    "agent:resume_strategy": null,
    "agent:session_id_field": null,
    "agent:prompt_arg_flag": null,
    // its conversation and state
    "agent:sessionid": null,
    "agent:last_failure": null,
    "agent:launch_blocked": null,
    "agent:pending_account": null,
    "agent:context": null,
};

/**
 * The keys of a previous occupant that a launch of a HOST agent must not inherit: the container ones. A launch
 * writes the image only when its agent has one, so without these a pane that ever held a container agent kept the
 * image and read as a container pane. (A launch into a clean pane deletes keys that are not there: no change.)
 */
export const CONTAINER_META_CLEARED_BY_A_HOST_LAUNCH: Record<string, null> = {
    "agent:container_image": null,
    "agent:container_volumes": null,
    "agent:container_command": null,
};

/**
 * Stop the CLI of the agent in `blockId`, then clear what the pane kept of it. The stop comes first and is best
 * effort (a pane that never started has nothing to stop); the meta is cleared whatever the stop did, so the pane
 * can always go back to the picker.
 */
export async function leavePaneAgent(
    blockId: string,
    deps: {
        stop: (blockId: string) => Promise<unknown>;
        setMeta: (meta: Record<string, unknown>) => Promise<unknown>;
        warn?: (message: string) => void;
    }
): Promise<void> {
    try {
        await deps.stop(blockId);
    } catch (e) {
        deps.warn?.(`stopping the CLI of ${blockId} before leaving its agent failed: ${String((e as Error)?.message ?? e)}`);
    }
    await deps.setMeta({ ...LEFT_AGENT_META });
}
